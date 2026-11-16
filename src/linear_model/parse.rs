//! Pure reference-aligned parsing. Remote strings never grant local authority.
use super::*;
use serde_json::Value;

fn text(value: &Value, max: usize) -> String {
    let mut units = 0;
    value
        .as_str()
        .unwrap_or("")
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= max
        })
        .collect()
}
fn id(value: &Value) -> Result<String, String> {
    let id = value["id"].as_str().ok_or("Missing Linear identifier")?;
    checked_id(id)?;
    Ok(id.into())
}
fn required(value: &Value, key: &str, max: usize) -> Result<String, String> {
    if !value[key].is_string() {
        return Err("Incomplete Linear object".into());
    }
    Ok(text(&value[key], max))
}
fn person(value: &Value, detailed: bool) -> Result<LinearPerson, String> {
    Ok(LinearPerson {
        id: id(value)?,
        name: required(value, "name", 500)?,
        display_name: detailed.then(|| text(&value["displayName"], 500)),
        email: detailed.then(|| text(&value["email"], 500)),
    })
}
pub(super) fn identity(value: &Value) -> Result<LinearIdentity, String> {
    let org = &value["organization"];
    Ok(LinearIdentity {
        viewer: person(&value["viewer"], true)?,
        organization: LinearOrganization {
            id: id(org)?,
            name: required(org, "name", 500)?,
            url_key: text(&org["urlKey"], 100),
        },
    })
}
pub(super) fn team(value: &Value) -> Result<LinearTeam, String> {
    Ok(LinearTeam {
        id: id(value)?,
        name: required(value, "name", 500)?,
        key: required(value, "key", 100)?,
    })
}
pub(super) fn state(value: &Value) -> Result<LinearState, String> {
    Ok(LinearState {
        id: id(value)?,
        name: required(value, "name", 500)?,
        r#type: required(value, "type", 50)?,
        color: text(&value["color"], 30),
        position: value["position"].as_f64().filter(|p| p.is_finite()),
    })
}
fn optional_person(value: &Value) -> Result<Option<LinearPerson>, String> {
    // Vue accepts a falsey optional person as absent, but validates every truthy one.
    let absent = match value {
        Value::Null => true,
        Value::Bool(b) => !*b,
        Value::Number(n) => n.as_f64() == Some(0.),
        Value::String(s) => s.is_empty(),
        _ => false,
    };
    if absent {
        Ok(None)
    } else {
        person(value, false).map(Some)
    }
}
pub(super) fn issue(value: &Value) -> Result<LinearIssue, String> {
    let priority = value["priority"]
        .as_f64()
        .filter(|p| p.fract() == 0. && (0. ..=4.).contains(p))
        .unwrap_or(0.) as u8;
    Ok(LinearIssue {
        id: id(value)?,
        identifier: required(value, "identifier", 100)?,
        title: required(value, "title", 2000)?,
        url: issue_url(value["url"].as_str().ok_or("Missing Linear issue URL")?)?,
        description: text(&value["description"], 200000),
        priority,
        created_at: text(&value["createdAt"], 50),
        updated_at: text(&value["updatedAt"], 50),
        state: state(&value["state"])?,
        team: team(&value["team"])?,
        assignee: optional_person(&value["assignee"])?,
        creator: optional_person(&value["creator"])?,
    })
}
pub(super) fn page<T>(
    value: &Value,
    mapper: impl Fn(&Value) -> Result<T, String>,
) -> Result<Page<T>, String> {
    let (nodes, has_next_page) = page_nodes(value)?;
    let items = nodes.iter().map(mapper).collect::<Result<Vec<_>, _>>()?;
    Ok(Page {
        items,
        has_next_page,
        end_cursor: value["pageInfo"]["endCursor"]
            .as_str()
            .map(|_| text(&value["pageInfo"]["endCursor"], 1024)),
    })
}
pub(super) fn updated_issue(value: &Value) -> Result<LinearIssue, String> {
    if value["issueUpdate"]["success"].as_bool() != Some(true) {
        return Err("Linear issue state update reported failure.".into());
    }
    issue(&value["issueUpdate"]["issue"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> Value {
        json!({"id":"issue-1","identifier":"DEV-1","title":"Fixture","url":"https://linear.app/team/issue/DEV-1",
        "description":"body","priority":1.0,"state":{"id":"state-1","name":"Working","type":"started","position":2.5,"color":"#fff"},
        "team":{"id":"team-1","name":"Team","key":"DEV"},"assignee":{"id":"user-1","name":"Owner"},"creator":null})
    }
    #[test]
    fn nested_fields_and_cursor_survive_without_silent_partial_lists() {
        let source = fixture();
        let result = issue(&source).unwrap();
        assert_eq!(result.priority, 1);
        assert_eq!(result.state.position, Some(2.5));
        assert_eq!(result.team.key, "DEV");
        assert_eq!(result.assignee.unwrap().id, "user-1");
        let mut list = json!({"nodes":[source.clone()],"pageInfo":{"hasNextPage":true,"endCursor":"cursor-2"}});
        let result = page(&list, issue).unwrap();
        assert!(result.has_next_page);
        assert_eq!(result.end_cursor.as_deref(), Some("cursor-2"));
        list["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"broken"}));
        assert!(page(&list, issue).is_err());
        for field in ["state", "team", "assignee"] {
            let mut broken = source.clone();
            broken[field] = json!({"id":"valid"});
            assert!(issue(&broken).is_err(), "{field}");
        }
        assert!(updated_issue(&json!({"issueUpdate":{"success":true}})).is_err());
        assert_eq!(
            updated_issue(&json!({"issueUpdate":{"success":true,"issue":source}}))
                .unwrap()
                .id,
            "issue-1"
        );
    }
    #[test]
    fn text_caps_preserve_utf8_and_reference_optional_semantics() {
        let mut source = fixture();
        source["title"] = json!("😀".repeat(1100));
        source["description"] = Value::Null;
        source["assignee"] = json!(false);
        source["creator"] = json!(0);
        source["priority"] = json!(4.1);
        let parsed = issue(&source).unwrap();
        assert_eq!(parsed.title.encode_utf16().count(), 2000);
        assert_eq!(parsed.priority, 0);
        assert!(
            parsed.description.is_empty() && parsed.assignee.is_none() && parsed.creator.is_none()
        );
        source["team"]["name"] = json!(false);
        assert!(issue(&source).is_err());
        assert_eq!(text(&json!("a😀"), 2), "a");
    }
    #[test]
    fn selected_filters_match_viewer_state_and_priority_without_overwriting_type() {
        let selection = IssueSelection {
            filter: LinearFilter::Open,
            team_id: Some("team-1".into()),
            state_id: Some("state-1".into()),
            priority: Some(0),
        };
        assert_eq!(
            issue_filter("viewer-1", &selection).unwrap(),
            json!({"team":{"id":{"eq":"team-1"}},"state":{"type":{"nin":["completed","canceled"]},"id":{"eq":"state-1"}},"priority":{"eq":0}})
        );
        assert_eq!(
            issue_filter("viewer-1", &IssueSelection::default()).unwrap(),
            json!({"assignee":{"id":{"eq":"viewer-1"}}})
        );
        assert_eq!(
            issue_filter(
                "",
                &IssueSelection {
                    filter: LinearFilter::Completed,
                    ..Default::default()
                }
            )
            .unwrap(),
            json!({"state":{"type":{"eq":"completed"}}})
        );
        assert!(issue_filter("", &IssueSelection::default()).is_err());
        assert!(
            issue_filter(
                "viewer-1",
                &IssueSelection {
                    priority: Some(5),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}
