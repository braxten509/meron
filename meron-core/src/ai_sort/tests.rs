//! Regression tests for ai sort.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;

fn folders() -> Vec<PromptFolder> {
    vec![
        PromptFolder {
            name: "Receipts".into(),
            note: "orders".into(),
            target: SortTarget::Server("Receipts".into()),
        },
        PromptFolder {
            name: "Work".into(),
            note: String::new(),
            target: SortTarget::Server("INBOX/Work".into()),
        },
    ]
}

fn targets(text: &str) -> Vec<Option<String>> {
    let ids: Vec<String> = (1..=4).map(|id| id.to_string()).collect();
    parse_answer(text, &ids, &folders())
        .into_values()
        .map(|(pick, _)| {
            pick.map(|pick| match pick.target {
                SortTarget::Server(folder) | SortTarget::Meron(folder) => folder,
            })
        })
        .collect()
}

#[test]
fn only_sure_picks_of_known_folders_move() {
    let good = r#"{"emails":[
        {"id":"1","folder":"Receipts","sure":true,"why":"order"},
        {"id":"2","folder":"Inbox","sure":true,"why":"unclear"},
        {"id":"3","folder":"Receipts","sure":false,"why":"guess"},
        {"id":"4","folder":"Holidays","sure":true,"why":"trip"}]}"#;
    assert_eq!(targets(good), [Some("Receipts".into()), None, None, None]);

    // Fenced, with text around it; a folder matched ignoring case; id 2
    // answered twice and 3 left out stay; an unknown id changes nothing.
    let fenced = "Here you go:\n```json\n{\"emails\":[\
        {\"id\":\"1\",\"folder\":\"work\",\"sure\":true,\"why\":\"\"},\
        {\"id\":\"2\",\"folder\":\"Work\",\"sure\":true,\"why\":\"\"},\
        {\"id\":\"2\",\"folder\":\"Work\",\"sure\":true,\"why\":\"\"},\
        {\"id\":\"9\",\"folder\":\"Work\",\"sure\":true,\"why\":\"\"}]}\n```";
    assert_eq!(
        targets(fenced),
        [Some("INBOX/Work".into()), None, None, None]
    );

    assert_eq!(targets("Sorry, I can't help."), [None, None, None, None]);
}

#[test]
fn spam_only_counts_with_a_backing_lesson_and_a_sure_answer() {
    let folders = folders();
    let spam = SpamChoice::new(&folders, "Junk").unwrap();
    let ids: Vec<String> = (1..=4).map(|id| id.to_string()).collect();
    let backed: HashSet<String> = ["1", "2", "4"].into_iter().map(String::from).collect();
    // Email 4 is bulk mail: its "best folder even when unsure" rule must
    // not reach Spam.
    let automated: HashSet<String> = HashSet::from(["4".to_string()]);
    let text = r#"{"emails":[
        {"id":"1","folder":"Spam","sure":true,"why":"same advert"},
        {"id":"2","folder":"Spam","sure":false,"why":"maybe"},
        {"id":"3","folder":"Spam","sure":true,"why":"looks spammy"},
        {"id":"4","folder":"Spam","sure":false,"why":"bulk"}]}"#;
    let picks = parse_answer_taught(text, &ids, &folders, &automated, Some((&spam, &backed)));
    let to: Vec<Option<SortTarget>> = picks
        .into_values()
        .map(|(pick, _)| pick.map(|pick| pick.target))
        .collect();
    // Only the backed email the AI is sure about goes to the Spam folder.
    assert_eq!(
        to,
        [Some(SortTarget::Server("Junk".into())), None, None, None]
    );
    // Without a lesson "Spam" is no answer at all, and the AI isn't offered it.
    assert!(
        parse_answer(text, &ids, &folders)
            .values()
            .all(|(pick, _)| pick.is_none())
    );
    assert!(
        !answer_schema_taught(&folders, None)
            .to_string()
            .contains("\"Spam\"")
    );
    assert!(
        answer_schema_taught(&folders, Some(&spam))
            .to_string()
            .contains("\"Spam\"")
    );
}

/// Keys kept in memory, as the keychain or keyed DB would.
struct KeyHost(std::sync::Mutex<HashMap<String, String>>);

impl crate::engine::EngineHost for KeyHost {
    fn open_db(&self) -> anyhow::Result<rusqlite::Connection> {
        crate::store::open_at(":memory:")
    }
    fn apply_secret(&self, _: &rusqlite::Connection, _: &str, _: &mut crate::imap::Creds) {}
    fn store_secret(
        &self,
        _: &rusqlite::Connection,
        _: &str,
        _: &crate::secrets::Secrets,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    fn load_named_secret(
        &self,
        _: &std::sync::Mutex<rusqlite::Connection>,
        name: &str,
    ) -> anyhow::Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(name).cloned())
    }
    fn store_named_secret(
        &self,
        _: &std::sync::Mutex<rusqlite::Connection>,
        name: &str,
        value: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut keys = self.0.lock().unwrap();
        match value {
            Some(value) => keys.insert(name.to_string(), value.to_string()),
            None => keys.remove(name),
        };
        Ok(())
    }
}

#[tokio::test]
async fn settings_show_only_a_hint_of_the_key() {
    let host = KeyHost(Default::default());
    let engine = Arc::new(crate::engine::Engine::new(Box::new(host)).unwrap());
    let long = "sk-or-v1-testvalue1234";
    dispatch(
        &engine,
        "setKey",
        &json!({ "provider": "openrouter", "key": long }),
    )
    .await
    .unwrap();
    dispatch(
        &engine,
        "setKey",
        &json!({ "provider": "tinfoil", "key": "abcd" }),
    )
    .await
    .unwrap();

    let got = dispatch(&engine, "get", &json!({})).await.unwrap();
    assert_eq!(got["keys"]["openrouter"]["key_hint"], "1234");
    assert_eq!(
        got["keys"]["tinfoil"],
        json!({ "has_key": true, "key_hint": null })
    );
    let text = got.to_string();
    assert!(!text.contains(long) && !text.contains("abcd"));
}
