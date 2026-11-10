//! One bulk Delete or Move of several conversations.
//!
//! Every picked conversation runs the ordinary single action, all at the same
//! time, as a *member* of one [`BulkRun`]. A single action talks to the mail
//! server in three steps: it takes the Meron-folder marks off, it moves the
//! mail, and it finds the copies the move made. A member that reaches one of
//! those steps waits there until every member still running has reached it
//! too; the step is then done once, for all of them together, and each member
//! carries on with its own share of the answer.
//!
//! So a run costs about as many trips to the server as one conversation does
//! alone, however many it holds, while what each conversation resolves, caches
//! and answers stays the single action's own code.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::join_all;
use serde_json::Value;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, oneshot};

use super::sync::{
    CopiesAsk, MovedCopies, Trashed, delete_to_trash_alone, move_on_server_alone,
    refresh_moved_copies_shared,
};
use super::Engine;
use crate::meron_marks::MarkJob;

/// How long the waiting members are held for one that is busy with something
/// of its own (a sender row searching the server, say). After this the step
/// goes ahead for those already waiting, and the late one gets its own turn.
const STRAGGLER_WAIT: Duration = Duration::from_secs(3);

/// How many members may use the server on their own at the same time. Mail
/// servers cap the connections of one account, often at ten.
const OWN_READS_AT_ONCE: usize = 3;

/// The server steps of a delete or move, in the order an action reaches them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    Started,
    Marks,
    Move,
    Copies,
}

/// Where a member's mail goes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MoveTo {
    /// The account's Trash, or gone for good from Drafts and Trash (see
    /// [`super::delete_to_trash`]).
    Trash,
    Folder(String),
}

/// One member's part of the move step.
pub(crate) struct MoveAsk {
    pub account: String,
    pub folder: String,
    pub to: MoveTo,
    pub uids: Vec<u32>,
}

/// What the move step answers, shared by every member of one move.
#[derive(Clone)]
pub(crate) enum Moved {
    /// See [`super::delete_to_trash`].
    Trashed(Option<Trashed>),
    /// The target's UIDNEXT from just before the move.
    Folder(Option<u32>),
}

/// A member waiting at a step: what it asked for, and where its answer goes.
struct Waiting<Ask, Answer> {
    ask: Ask,
    answer: oneshot::Sender<Answer>,
}

#[derive(Default)]
struct State {
    next_member: u64,
    /// The members still running, each with the latest step it reached.
    members: HashMap<u64, Step>,
    marks: Vec<Waiting<Vec<MarkJob>, ()>>,
    moves: Vec<Waiting<MoveAsk, anyhow::Result<Moved>>>,
    copies: Vec<Waiting<CopiesAsk, MovedCopies>>,
    /// Set when nothing does the steps any more: nobody may wait.
    closed: bool,
}

/// The members waiting at one step, taken out to be answered.
enum Due {
    Marks(Vec<Waiting<Vec<MarkJob>, ()>>),
    Moves(Vec<Waiting<MoveAsk, anyhow::Result<Moved>>>),
    Copies(Vec<Waiting<CopiesAsk, MovedCopies>>),
    /// Somebody has yet to reach the step the others wait at.
    Nothing,
    /// Every member has finished.
    Finished,
}

/// The shared state of one bulk run. See the module's opening note.
pub struct BulkRun {
    state: Mutex<State>,
    changed: Notify,
    own_reads: Arc<Semaphore>,
}

/// One member's place in a [`BulkRun`]. Dropping it (the action finished,
/// failed or was cancelled) takes the member out, so nobody waits for it.
pub struct Seat {
    run: Arc<BulkRun>,
    id: u64,
}

tokio::task_local! {
    /// The seat of the action this task is running, inside a bulk run.
    static SEAT: Arc<Seat>;
}

thread_local! {
    /// The seat of the action this thread is running (the phone's blocking
    /// commands): [`with_thread_seat`] hands it to each future they block on.
    static THREAD_SEAT: std::cell::RefCell<Option<Arc<Seat>>> =
        const { std::cell::RefCell::new(None) };
}

/// The seat of the action being run right now, when it is part of a bulk run.
pub(crate) fn seat() -> Option<Arc<Seat>> {
    SEAT.try_with(Arc::clone).ok()
}

/// Run `future` as the member whose seat this thread holds, if it holds one.
pub(crate) async fn with_thread_seat<F: Future>(future: F) -> F::Output {
    match THREAD_SEAT.with(|seat| seat.borrow().clone()) {
        Some(seat) => SEAT.scope(seat, future).await,
        None => future.await,
    }
}

/// A turn at using the server on a member's own (a read or an upload that is
/// not one of the shared steps). `None` outside a bulk run, where nothing
/// needs rationing. Give it back before waiting at a shared step: a member
/// holding one there would keep the others from reaching the step.
pub(crate) async fn own_read_turn() -> Option<OwnedSemaphorePermit> {
    let seat = seat()?;
    seat.run.own_reads.clone().acquire_owned().await.ok()
}

impl BulkRun {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            changed: Notify::new(),
            own_reads: Arc::new(Semaphore::new(OWN_READS_AT_ONCE)),
        })
    }

    /// A seat for one more member. Every member is seated before any of them
    /// starts, so the first to reach a step still waits for the others.
    fn seat(self: &Arc<Self>) -> Arc<Seat> {
        let mut state = self.state.lock().unwrap();
        let id = state.next_member;
        state.next_member += 1;
        state.members.insert(id, Step::Started);
        Arc::new(Seat {
            run: self.clone(),
            id,
        })
    }

    /// The step every running member has reached, and the members waiting at
    /// it. With `overdue`, the earliest step anyone waits at, even though
    /// somebody has yet to reach it.
    fn take_due(&self, overdue: bool) -> Due {
        let mut state = self.state.lock().unwrap();
        let Some(reached) = state.members.values().min().copied() else {
            return Due::Finished;
        };
        let due = |step: Step, waiting: bool| waiting && (overdue || reached >= step);
        if due(Step::Marks, !state.marks.is_empty()) {
            Due::Marks(std::mem::take(&mut state.marks))
        } else if due(Step::Move, !state.moves.is_empty()) {
            Due::Moves(std::mem::take(&mut state.moves))
        } else if due(Step::Copies, !state.copies.is_empty()) {
            Due::Copies(std::mem::take(&mut state.copies))
        } else {
            Due::Nothing
        }
    }

    /// Do the shared steps for the members until every one has finished.
    async fn drive(&self, engine: &Arc<Engine>) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let mut due = self.take_due(false);
            if matches!(due, Due::Nothing)
                && tokio::time::timeout(STRAGGLER_WAIT, changed).await.is_err()
            {
                due = self.take_due(true);
            }
            match due {
                Due::Finished => return,
                Due::Nothing => {}
                Due::Marks(waiting) => answer_marks(engine, waiting).await,
                Due::Moves(waiting) => answer_moves(engine, waiting).await,
                Due::Copies(waiting) => answer_copies(engine, waiting).await,
            }
        }
    }

    /// Nothing does the steps any more: let go of whoever waits, and let
    /// nobody wait from here on.
    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.marks.clear();
        state.moves.clear();
        state.copies.clear();
    }
}

impl Seat {
    /// Wait at `step` with `ask` until the step is done for everyone. `None`
    /// when the run has stopped doing the steps.
    async fn wait<Ask, Answer>(
        &self,
        step: Step,
        queue: fn(&mut State) -> &mut Vec<Waiting<Ask, Answer>>,
        ask: Ask,
    ) -> Option<Answer> {
        let (answer, answered) = oneshot::channel();
        {
            let mut state = self.run.state.lock().unwrap();
            if state.closed {
                return None;
            }
            if let Some(reached) = state.members.get_mut(&self.id) {
                *reached = (*reached).max(step);
            }
            queue(&mut state).push(Waiting { ask, answer });
        }
        self.run.changed.notify_one();
        answered.await.ok()
    }

    /// The marks step: write `jobs` together with every other member's.
    pub(crate) async fn marks(&self, jobs: Vec<MarkJob>) {
        self.wait(Step::Marks, |state| &mut state.marks, jobs).await;
    }

    /// The move step: move this member's mail in the same command as every
    /// other member's from the same folder to the same place.
    pub(crate) async fn moved(&self, ask: MoveAsk) -> anyhow::Result<Moved> {
        self.wait(Step::Move, |state| &mut state.moves, ask)
            .await
            .unwrap_or_else(|| Err(anyhow::anyhow!("the bulk action stopped before this move")))
    }

    /// The copies step: find this member's moved copies in the one read of
    /// the target folder that serves every member. `None` when the run has
    /// stopped doing the steps.
    pub(crate) async fn copies(&self, ask: CopiesAsk) -> Option<MovedCopies> {
        self.wait(Step::Copies, |state| &mut state.copies, ask).await
    }

    /// Take the member out of the run.
    fn leave(&self) {
        self.run.state.lock().unwrap().members.remove(&self.id);
        self.run.changed.notify_one();
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.leave();
    }
}

async fn answer_marks(engine: &Arc<Engine>, waiting: Vec<Waiting<Vec<MarkJob>, ()>>) {
    let mut jobs = Vec::new();
    let mut answers = Vec::new();
    for member in waiting {
        jobs.extend(member.ask);
        answers.push(member.answer);
    }
    crate::meron_marks::write_all_alone(engine, jobs).await;
    for answer in answers {
        let _ = answer.send(());
    }
}

/// An error several members share. Shows like the error it holds, causes
/// included.
#[derive(Debug)]
struct SharedError(Arc<anyhow::Error>);

impl std::fmt::Display for SharedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SharedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

async fn answer_moves(engine: &Arc<Engine>, waiting: Vec<Waiting<MoveAsk, anyhow::Result<Moved>>>) {
    // One command per (account, source folder, destination).
    let mut groups: Vec<Vec<Waiting<MoveAsk, anyhow::Result<Moved>>>> = Vec::new();
    for member in waiting {
        let same = |group: &&mut Vec<Waiting<MoveAsk, anyhow::Result<Moved>>>| {
            let first = &group[0].ask;
            first.account == member.ask.account
                && first.folder == member.ask.folder
                && first.to == member.ask.to
        };
        match groups.iter_mut().find(same) {
            Some(group) => group.push(member),
            None => groups.push(vec![member]),
        }
    }
    join_all(groups.into_iter().map(|group| async move {
        let first = &group[0].ask;
        let mut uids: Vec<u32> = group.iter().flat_map(|member| member.ask.uids.iter().copied()).collect();
        uids.sort_unstable();
        uids.dedup();
        let moved = match &first.to {
            MoveTo::Trash => delete_to_trash_alone(engine, &first.account, &first.folder, &uids)
                .await
                .map(Moved::Trashed),
            MoveTo::Folder(target) => {
                move_on_server_alone(engine, &first.account, &first.folder, target, &uids)
                    .await
                    .map(Moved::Folder)
            }
        };
        match moved {
            Ok(moved) => {
                for member in group {
                    let _ = member.answer.send(Ok(moved.clone()));
                }
            }
            Err(err) => {
                let err = Arc::new(err);
                for member in group {
                    let _ = member.answer.send(Err(anyhow::Error::new(SharedError(err.clone()))));
                }
            }
        }
    }))
    .await;
}

async fn answer_copies(engine: &Arc<Engine>, waiting: Vec<Waiting<CopiesAsk, MovedCopies>>) {
    // One read per (account, target folder).
    let mut groups: Vec<Vec<Waiting<CopiesAsk, MovedCopies>>> = Vec::new();
    for member in waiting {
        let same = |group: &&mut Vec<Waiting<CopiesAsk, MovedCopies>>| {
            group[0].ask.account == member.ask.account && group[0].ask.target == member.ask.target
        };
        match groups.iter_mut().find(same) {
            Some(group) => group.push(member),
            None => groups.push(vec![member]),
        }
    }
    join_all(groups.into_iter().map(|group| async move {
        let (asks, answers): (Vec<CopiesAsk>, Vec<oneshot::Sender<MovedCopies>>) =
            group.into_iter().map(|member| (member.ask, member.answer)).unzip();
        let copies = refresh_moved_copies_shared(engine, &asks).await;
        for (answer, copies) in answers.into_iter().zip(copies) {
            let _ = answer.send(copies);
        }
    }))
    .await;
}

/// Run `actions` (the single action of each picked conversation) as one bulk
/// run and give back their answers, in order. One action alone just runs.
pub async fn run_together<F: Future>(engine: &Arc<Engine>, mut actions: Vec<F>) -> Vec<F::Output> {
    if actions.len() == 1 {
        return vec![actions.remove(0).await];
    }
    let run = BulkRun::new();
    let seats: Vec<Arc<Seat>> = actions.iter().map(|_| run.seat()).collect();
    let members = join_all(actions.into_iter().zip(seats).map(|(action, seat)| async move {
        let answer = SEAT.scope(seat.clone(), action).await;
        seat.leave();
        answer
    }));
    let (answers, ()) = tokio::join!(members, run.drive(engine));
    run.close();
    answers
}

/// [`run_together`] for the phone's blocking commands: `action(index)` is
/// the single action of the `index`th of `count` conversations, and each runs
/// on a thread of its own.
pub(crate) fn run_together_blocking(
    engine: &Arc<Engine>,
    count: usize,
    action: impl Fn(usize) -> Result<Value, String> + Sync,
) -> Vec<Result<Value, String>> {
    if count <= 1 {
        return (0..count).map(action).collect();
    }
    let run = BulkRun::new();
    let seats: Vec<Arc<Seat>> = (0..count).map(|_| run.seat()).collect();
    let action = &action;
    std::thread::scope(|scope| {
        let members: Vec<_> = seats
            .into_iter()
            .enumerate()
            .map(|(index, seat)| {
                scope.spawn(move || {
                    THREAD_SEAT.with(|held| *held.borrow_mut() = Some(seat.clone()));
                    let answer = action(index);
                    THREAD_SEAT.with(|held| held.borrow_mut().take());
                    seat.leave();
                    answer
                })
            })
            .collect();
        let driven = crate::ffi::engine_block_on_anyhow(async {
            run.drive(engine).await;
            anyhow::Ok(())
        });
        if let Err(err) = driven {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "bulk action could not share its server steps: {err:#}"
            );
        }
        run.close();
        members
            .into_iter()
            .map(|member| {
                member
                    .join()
                    .unwrap_or_else(|_| Err("the action stopped unexpectedly".to_string()))
            })
            .collect()
    })
}
