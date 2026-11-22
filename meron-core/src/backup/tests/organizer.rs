//! Calendar and task records in a portable backup.
//! Fixtures include repeated imports and distinct tasks with matching titles and dates.
//! A backup created before task support still restores successfully.

use super::*;

/// Tasks have no server copy, so a backup is the only thing standing between a
/// reinstall and losing them. Everything the user typed has to survive the
/// round trip, including the link back to a mail thread.
#[test]
fn backup_round_trips_task_lists() {
    let source = test_conn();
    let list = crate::tasks::ensure_default_list(&source).unwrap();
    let groceries = store::insert_task_list(&source, "list-groceries", "Groceries").unwrap();
    crate::tasks::create_task(
        &source,
        &list,
        "Reply to the landlord",
        "about the boiler",
        1_700_000_000,
        "acct1",
        "acct1#thread#4",
        "<boiler@example.com>",
    )
    .unwrap();
    let timed_id = store::tasks_for_list(&source, &list, true).unwrap()[0]
        .id
        .clone();
    store::set_task_due_time(&source, &timed_id, true).unwrap();
    let done =
        crate::tasks::create_task(&source, &groceries.id, "Milk", "", 0, "", "", "").unwrap();
    crate::tasks::set_done(&source, done["task"]["id"].as_str().unwrap(), true).unwrap();

    let data = collect(&source, false, &no_secrets).unwrap();
    assert_eq!(data.task_lists.len(), 2);

    let target = test_conn();
    let summary = apply(&target, &data, &|_, _, _| Ok(())).unwrap();
    assert_eq!(summary.tasks, 2);

    let restored_lists = store::task_lists(&target).unwrap();
    let titles: Vec<&str> = restored_lists
        .iter()
        .map(|list| list.title.as_str())
        .collect();
    assert!(titles.contains(&"Groceries"), "{titles:?}");

    let my_tasks = restored_lists
        .iter()
        .find(|list| list.title == crate::tasks::DEFAULT_LIST_TITLE)
        .unwrap();
    let tasks = store::tasks_for_list(&target, &my_tasks.id, true).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Reply to the landlord");
    assert_eq!(tasks[0].notes, "about the boiler");
    assert_eq!(tasks[0].due_at, 1_700_000_000);
    assert!(tasks[0].due_has_time);
    assert_eq!(tasks[0].thread_id, "acct1#thread#4");
    assert_eq!(tasks[0].message_id, "<boiler@example.com>");

    let groceries_id = &restored_lists
        .iter()
        .find(|list| list.title == "Groceries")
        .unwrap()
        .id;
    let milk = store::tasks_for_list(&target, groceries_id, true).unwrap();
    assert!(
        milk[0].completed_at > 0,
        "completion survives the round trip"
    );
}

/// Importing the same file twice is something people do — twice through the
/// dialog, or onto a device that already restored it. It must not double the
/// list or its contents.
#[test]
fn importing_a_backup_twice_does_not_duplicate_tasks() {
    let source = test_conn();
    let list = crate::tasks::ensure_default_list(&source).unwrap();
    crate::tasks::create_task(&source, &list, "Book the flight", "", 0, "", "", "").unwrap();
    let data = collect(&source, false, &no_secrets).unwrap();

    let target = test_conn();
    // The destination has already opened Tasks once, so the default list exists
    // and must remain separate from the imported list.
    crate::tasks::ensure_default_list(&target).unwrap();

    assert_eq!(apply(&target, &data, &|_, _, _| Ok(())).unwrap().tasks, 1);
    assert_eq!(
        apply(&target, &data, &|_, _, _| Ok(())).unwrap().tasks,
        0,
        "the second import adds nothing"
    );

    let lists = store::task_lists(&target).unwrap();
    assert_eq!(lists.len(), 2, "preserve the unrelated existing list");
    assert_eq!(
        store::tasks_for_list(&target, &list, true).unwrap().len(),
        1
    );
}

/// Files written before Tasks existed carry no `task_lists` key at all.
#[test]
fn a_backup_without_tasks_still_imports() {
    let conn = test_conn();
    let data: BackupData =
        serde_json::from_str(r#"{"accounts":[],"settings":{"theme_id":"meron-light"}}"#).unwrap();
    let summary = apply(&conn, &data, &|_, _, _| Ok(())).unwrap();
    assert_eq!(summary.tasks, 0);
    assert_eq!(summary.settings, 1);
    assert!(store::task_lists(&conn).unwrap().is_empty());
}

#[test]
fn task_backup_preserves_distinct_rows_with_matching_titles_and_timestamps() {
    let source = test_conn();
    for list_id in ["list-a", "list-b"] {
        store::insert_task_list(&source, list_id, "Same list").unwrap();
        for suffix in ["a", "b"] {
            store::insert_task(
                &source,
                &format!("{list_id}-{suffix}"),
                list_id,
                "Same task",
                suffix,
                0,
                "",
                "",
                "",
            )
            .unwrap();
        }
    }
    source
        .execute("UPDATE tasks SET created_at = 100", [])
        .unwrap();
    let mut data = collect(&source, false, &no_secrets).unwrap();
    for legacy in [false, true] {
        if legacy {
            for list in &mut data.task_lists {
                list.id.clear();
                for task in &mut list.tasks {
                    task.id.clear();
                }
            }
        }
        let target = test_conn();
        assert_eq!(apply(&target, &data, &|_, _, _| Ok(())).unwrap().tasks, 4);
        assert_eq!(apply(&target, &data, &|_, _, _| Ok(())).unwrap().tasks, 0);
        let lists = store::task_lists(&target).unwrap();
        assert_eq!(lists.len(), 2);
        for list in lists {
            assert_eq!(
                store::tasks_for_list(&target, &list.id, true)
                    .unwrap()
                    .len(),
                2
            );
        }
    }
}

/// Calendar events are local-only like tasks, so the backup carries them; a
/// second import adds nothing, and an event edited since the export keeps the
/// edit.
#[test]
fn calendar_events_round_trip_once() {
    let source = test_conn();
    let made = crate::calendar::events::create_event(
        &source,
        &crate::calendar::events::EventInput {
            title: "Dentist".into(),
            start: 1_790_812_800,
            end: 1_790_816_400,
            repeat: "yearly".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let id = made["event"]["id"].as_str().unwrap().to_string();
    let data = collect(&source, false, &no_secrets).unwrap();

    let target = test_conn();
    assert_eq!(apply(&target, &data, &|_, _, _| Ok(())).unwrap().events, 1);
    let restored = store::event(&target, &id).unwrap().unwrap();
    assert_eq!(restored.title, "Dentist");
    assert_eq!(restored.rrule, "yearly");
    assert!(restored.recurring);

    target
        .execute(
            "UPDATE events SET title = 'Dentist (moved)' WHERE id = ?1",
            [&id],
        )
        .unwrap();
    assert_eq!(apply(&target, &data, &|_, _, _| Ok(())).unwrap().events, 0);
    assert_eq!(
        store::event(&target, &id).unwrap().unwrap().title,
        "Dentist (moved)"
    );
}
