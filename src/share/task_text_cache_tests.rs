use super::*;

#[test]
fn shared_board_title_cache_reuses_unchanged_cards_and_drops_old_entries() {
    let projects: Vec<_> = (1..=3)
        .map(|id| SharedProject {
            id,
            name: format!("Project {id}"),
            path: format!("C:/fixture/{id}").into(),
        })
        .collect();
    let mut cards: Vec<_> = (1..=1500)
        .map(|id| SharedTask {
            id: id.to_string(),
            project_id: (id - 1) / 500 + 1,
            title: format!("Card {id}: password=synthetic"),
            column: "todo".into(),
            guest_by: None,
        })
        .collect();
    let mut board = TaskBoard::default();
    board.publish(cards.clone(), &projects);
    let previous = board.text_cache.clone();
    assert_eq!(previous.len(), 1500);
    cards[700].title = "Changed password=synthetic2".into();
    board.publish(cards.clone(), &projects);
    let reused = previous
        .iter()
        .filter(|(id, value)| Arc::ptr_eq(value, &board.text_cache[*id]))
        .count();
    assert_eq!(reused, 1499);
    assert_eq!(board.cards[700].title, "Changed password=••••");
    cards[701].guest_by = Some("token=fixture".into());
    let old = board.text_cache["702"].clone();
    board.publish(cards.clone(), &projects);
    assert!(!Arc::ptr_eq(&old, &board.text_cache["702"]));
    assert_eq!(board.cards[701].guest_by.as_deref(), Some("token=••••"));
    cards.truncate(1);
    board.publish(cards, &projects);
    assert_eq!(board.text_cache.len(), 1);
    board.invalidate();
    assert!(board.text_cache.is_empty());
}
