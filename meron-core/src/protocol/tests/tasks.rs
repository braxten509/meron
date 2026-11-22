//! Task commands exposed through the mobile protocol.
//! Cases exercise returned task data and missing parameters.
//! The protocol stub also answers when a data directory is unavailable.

use super::*;

#[test]
fn mobile_protocol_serves_the_tasks_surface() {
    let data_directory = unique_data_directory("tasks");
    let directory = data_directory.to_str().unwrap().to_string();

    // Reading lists on a fresh install bootstraps one, so the screen is never
    // an empty shell with nowhere to type.
    let lists = invoke_mobile_protocol_json(
        r#"{"id":300,"method":"tasks.lists","params":{}}"#,
        Some(&directory),
    );
    assert_eq!(lists["id"], 300);
    let list_id = lists["result"]["default_list_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!list_id.is_empty(), "{lists}");
    assert_eq!(lists["result"]["lists"].as_array().unwrap().len(), 1);

    let created = invoke_mobile_protocol_json(
        &format!(
            r#"{{"id":301,"method":"tasks.create","params":{{"list_id":"{list_id}","title":"Book the flight","thread_id":"acct#t1","account":"acct"}}}}"#
        ),
        Some(&directory),
    );
    let task_id = created["result"]["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(created["result"]["task"]["title"], "Book the flight");
    assert_eq!(created["result"]["task"]["done"], false);

    let linked = invoke_mobile_protocol_json(
        r#"{"id":302,"method":"tasks.forThread","params":{"thread_id":"acct#t1"}}"#,
        Some(&directory),
    );
    assert_eq!(linked["result"]["tasks"][0]["id"], task_id.as_str());

    let done = invoke_mobile_protocol_json(
        &format!(
            r#"{{"id":303,"method":"tasks.setDone","params":{{"task_id":"{task_id}","done":true}}}}"#
        ),
        Some(&directory),
    );
    assert_eq!(done["result"]["task"]["done"], true);

    // Completed tasks are hidden by default and visible on request.
    let active = invoke_mobile_protocol_json(
        &format!(r#"{{"id":304,"method":"tasks.items","params":{{"list_id":"{list_id}"}}}}"#),
        Some(&directory),
    );
    assert!(
        active["result"]["tasks"].as_array().unwrap().is_empty(),
        "{active}"
    );
    let all = invoke_mobile_protocol_json(
        &format!(
            r#"{{"id":305,"method":"tasks.items","params":{{"list_id":"{list_id}","include_completed":true}}}}"#
        ),
        Some(&directory),
    );
    assert_eq!(all["result"]["tasks"].as_array().unwrap().len(), 1);

    let cleared = invoke_mobile_protocol_json(
        &format!(
            r#"{{"id":306,"method":"tasks.clearCompleted","params":{{"list_id":"{list_id}"}}}}"#
        ),
        Some(&directory),
    );
    assert_eq!(cleared["result"]["removed"], 1);

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_reports_missing_task_parameters() {
    let data_directory = unique_data_directory("tasks-invalid");
    let directory = data_directory.to_str().unwrap().to_string();
    let value = invoke_mobile_protocol_json(
        r#"{"id":310,"method":"tasks.items","params":{}}"#,
        Some(&directory),
    );
    assert_eq!(value["error"]["message"], "list_id required", "{value}");
    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn protocol_stub_answers_tasks_without_a_data_directory() {
    let value = invoke_protocol_json(r#"{"id":311,"method":"tasks.lists","params":{}}"#);
    assert_eq!(
        value["result"],
        json!({ "lists": [], "default_list_id": "" })
    );
    let unknown = invoke_protocol_json(r#"{"id":312,"method":"tasks.nope","params":{}}"#);
    assert_eq!(unknown["error"]["message"], "unknown method: tasks.nope");
}
