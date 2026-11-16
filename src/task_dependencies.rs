//! Bounded task prerequisites. Missing/deleted cards remain blockers.
use crate::workspace::Task;
use std::collections::{HashMap, HashSet, VecDeque};

pub const MAX_DEPS: usize = 10;

pub fn waiting_on(task: &Task, tasks: &[Task]) -> Vec<u64> {
    Index::new(tasks).waiting_on(task)
}

pub struct Index<'a> {
    tasks: HashMap<u64, &'a Task>,
}
impl<'a> Index<'a> {
    pub fn new(tasks: &'a [Task]) -> Self {
        Self {
            tasks: tasks.iter().map(|task| (task.id, task)).collect(),
        }
    }
    pub fn task(&self, id: u64, project: u64) -> Option<&'a Task> {
        self.tasks
            .get(&id)
            .copied()
            .filter(|task| task.project == project)
    }
    pub fn waiting_on(&self, task: &Task) -> Vec<u64> {
        task.deps
            .iter()
            .copied()
            .filter(|id| {
                !self
                    .task(*id, task.project)
                    .is_some_and(|other| other.column == 3)
            })
            .collect()
    }
}

pub fn validate(tasks: &[Task], next_id: u64) -> Result<(), String> {
    let mut by_id = HashMap::new();
    for task in tasks {
        if by_id.insert(task.id, task).is_some() {
            return Err("Duplicate task identifier.".into());
        }
    }
    let mut edges: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut degrees: HashMap<u64, usize> = tasks.iter().map(|task| (task.id, 0)).collect();
    for task in tasks {
        if task.deps.len() > MAX_DEPS {
            return Err("A task can wait for at most ten cards.".into());
        }
        let mut seen = HashSet::new();
        for id in &task.deps {
            if *id == 0 || *id >= next_id || *id == task.id || !seen.insert(*id) {
                return Err("Invalid task prerequisite identifier.".into());
            }
            if let Some(other) = by_id.get(id) {
                if other.project != task.project {
                    return Err("Task prerequisites must belong to the same project.".into());
                }
                edges.entry(*id).or_default().push(task.id);
                *degrees.get_mut(&task.id).unwrap() += 1;
            }
        }
    }
    // Iterative topological traversal avoids stack exhaustion from imported chains.
    let mut ready: VecDeque<_> = degrees
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop_front() {
        visited += 1;
        for dependant in edges.get(&id).into_iter().flatten() {
            let degree = degrees.get_mut(dependant).unwrap();
            *degree -= 1;
            if *degree == 0 {
                ready.push_back(*dependant);
            }
        }
    }
    if visited != tasks.len() {
        return Err("Task prerequisites contain a cycle.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task(id: u64, deps: Vec<u64>) -> Task {
        Task {
            created_by: None,
            report: None,
            last_waiting: Vec::new(),
            id,
            project: 1,
            title: "Synthetic".into(),
            column: 0,
            guest_by: None,
            timeline: Default::default(),
            deps,
        }
    }
    #[test]
    fn deleted_and_reopened_prerequisites_still_block() {
        let mut tasks = vec![task(1, vec![]), task(2, vec![1])];
        assert_eq!(waiting_on(&tasks[1], &tasks), vec![1]);
        tasks[0].column = 3;
        assert!(waiting_on(&tasks[1], &tasks).is_empty());
        tasks[0].column = 0;
        assert_eq!(waiting_on(&tasks[1], &tasks), vec![1]);
        tasks.remove(0);
        validate(&tasks, 3).unwrap();
        assert_eq!(waiting_on(&tasks[0], &tasks), vec![1]);
    }
    #[test]
    fn imported_graph_rejects_cycles_cross_project_and_invalid_links() {
        for deps in [vec![0], vec![2], vec![3], vec![1, 1], vec![1; 11]] {
            assert!(validate(&[task(1, vec![]), task(2, deps)], 3).is_err());
        }
        assert!(validate(&[task(1, vec![2]), task(2, vec![1])], 3).is_err());
        let mut other = task(1, vec![]);
        other.project = 2;
        assert!(validate(&[other, task(2, vec![1])], 3).is_err());
    }
    #[test]
    fn old_cards_default_to_no_dependencies_and_long_chains_are_iterative() {
        let old: Task = serde_json::from_value(
            serde_json::json!({"id":1,"project":1,"title":"Old","column":0}),
        )
        .unwrap();
        assert!(old.deps.is_empty());
        let tasks: Vec<_> = (1..=10_000)
            .map(|id| task(id, if id == 1 { vec![] } else { vec![id - 1] }))
            .collect();
        validate(&tasks, 10_001).unwrap();
    }
}
