use std::collections::{HashMap, HashSet, VecDeque};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParError {
    #[error("cycle detected involving task: {0}")]
    CycleDetected(String),
    #[error("unknown dependency '{dep}' in task '{task}'")]
    UnknownDep { task: String, dep: String },
    #[error("task '{0}' failed with exit code {1}")]
    TaskFailed(String, i32),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid task line: {0}")]
    InvalidLine(String),
}

/// A task in the DAG.
#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub deps: Vec<String>,
    pub command: String,
}

/// Parse a task line: `ID\tDEPS\tCOMMAND` (deps comma-separated, empty for none).
pub fn parse_task_line(line: &str) -> Result<Task, ParError> {
    let parts: Vec<&str> = line.splitn(3, '\t').collect();
    if parts.len() < 3 {
        return Err(ParError::InvalidLine(line.to_string()));
    }
    let id = parts[0].to_string();
    let deps = if parts[1].is_empty() {
        Vec::new()
    } else {
        parts[1].split(',').map(|s| s.trim().to_string()).collect()
    };
    let command = parts[2].to_string();
    Ok(Task { id, deps, command })
}

/// Validate the task DAG: check for missing deps and cycles.
pub fn validate_dag(tasks: &[Task]) -> Result<(), ParError> {
    let ids: HashSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();

    // Check for unknown deps
    for task in tasks {
        for dep in &task.deps {
            if !ids.contains(dep.as_str()) {
                return Err(ParError::UnknownDep {
                    task: task.id.clone(),
                    dep: dep.clone(),
                });
            }
        }
    }

    // Topological sort to detect cycles (Kahn's algorithm)
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for task in tasks {
        in_degree.entry(task.id.as_str()).or_insert(0);
        for dep in &task.deps {
            adj.entry(dep.as_str())
                .or_default()
                .push(task.id.as_str());
            *in_degree.entry(task.id.as_str()).or_insert(0) += 1;
        }
    }

    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|&(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut visited = 0usize;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        if let Some(neighbors) = adj.get(node) {
            for &next in neighbors {
                let deg = in_degree.get_mut(next).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(next);
                }
            }
        }
    }

    if visited != tasks.len() {
        // Find a task in the cycle
        let in_cycle = in_degree
            .iter()
            .find(|&(_, &deg)| deg > 0)
            .map(|(&id, _)| id)
            .unwrap_or("unknown");
        return Err(ParError::CycleDetected(in_cycle.to_string()));
    }

    Ok(())
}

/// Execute tasks in parallel respecting dependencies.
pub fn execute_par(
    tasks: &[Task],
    parallelism: usize,
    keep_going: bool,
    retry: usize,
    prefix_output: bool,
) -> Result<Vec<TaskResult>, ParError> {
    validate_dag(tasks)?;

    let task_map: HashMap<&str, &Task> = tasks.iter().map(|t| (t.id.as_str(), t)).collect();
    let results: Arc<Mutex<Vec<TaskResult>>> = Arc::new(Mutex::new(Vec::new()));
    let completed: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let failed: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Build reverse dep count
    let mut remaining_deps: HashMap<String, usize> = HashMap::new();
    for task in tasks {
        remaining_deps.insert(task.id.clone(), task.deps.len());
    }
    let remaining_deps = Arc::new(Mutex::new(remaining_deps));

    // Ready queue: tasks with 0 remaining deps
    let ready: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
    {
        let rd = remaining_deps.lock().unwrap();
        for task in tasks {
            if rd[&task.id] == 0 {
                ready.lock().unwrap().push_back(task.id.clone());
            }
        }
    }

    let total = tasks.len();
    let done_count = Arc::new(Mutex::new(0usize));

    // Worker pool
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let rx = Arc::new(Mutex::new(rx));

    // Seed the channel with initially ready tasks
    {
        let ready_q = ready.lock().unwrap();
        for id in ready_q.iter() {
            tx.send(id.clone()).unwrap();
        }
    }

    let mut handles = Vec::new();
    for _ in 0..parallelism.min(total) {
        let rx = Arc::clone(&rx);
        let tx = tx.clone();
        let results = Arc::clone(&results);
        let completed = Arc::clone(&completed);
        let failed = Arc::clone(&failed);
        let remaining_deps = Arc::clone(&remaining_deps);
        let done_count = Arc::clone(&done_count);
        let task_map: HashMap<String, Task> = task_map
            .iter()
            .map(|(k, v)| (k.to_string(), (*v).clone()))
            .collect();
        let total = total;

        let handle = thread::spawn(move || {
            loop {
                let task_id = {
                    let rx = rx.lock().unwrap();
                    match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(id) => id,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            let done = *done_count.lock().unwrap();
                            if done >= total {
                                return;
                            }
                            if !keep_going && !failed.lock().unwrap().is_empty() {
                                return;
                            }
                            continue;
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                };

                // Check if we should stop
                if !keep_going && !failed.lock().unwrap().is_empty() {
                    *done_count.lock().unwrap() += 1;
                    return;
                }

                let task = match task_map.get(&task_id) {
                    Some(t) => t,
                    None => continue,
                };

                // Execute with retries
                let mut last_exit = 0i32;
                let mut success = false;
                for attempt in 0..=retry {
                    if prefix_output && attempt > 0 {
                        eprintln!("[{task_id}] retry {attempt}/{retry}");
                    }

                    match Command::new("sh").arg("-c").arg(&task.command).status() {
                        Ok(status) => {
                            last_exit = status.code().unwrap_or(1);
                            if status.success() {
                                success = true;
                                break;
                            }
                        }
                        Err(_) => {
                            last_exit = 127;
                        }
                    }
                }

                let result = TaskResult {
                    id: task_id.clone(),
                    success,
                    exit_code: last_exit,
                };
                results.lock().unwrap().push(result);

                if success {
                    completed.lock().unwrap().insert(task_id.clone());

                    // Unlock dependents
                    let mut rd = remaining_deps.lock().unwrap();
                    for (id, count) in rd.iter_mut() {
                        let t = task_map.get(id).unwrap();
                        if t.deps.contains(&task_id) {
                            *count -= 1;
                            if *count == 0 {
                                let _ = tx.send(id.clone());
                            }
                        }
                    }
                } else {
                    failed.lock().unwrap().insert(task_id.clone());
                }

                *done_count.lock().unwrap() += 1;
                let done = *done_count.lock().unwrap();
                if done >= total {
                    return;
                }
            }
        });
        handles.push(handle);
    }

    drop(tx); // Drop sender so receivers can detect completion

    for handle in handles {
        let _ = handle.join();
    }

    let results = Arc::try_unwrap(results).unwrap().into_inner().unwrap();
    Ok(results)
}

#[derive(Debug, Clone)]
pub struct TaskResult {
    pub id: String,
    pub success: bool,
    pub exit_code: i32,
}

/// Compute topological order for dry-run output.
pub fn topological_order(tasks: &[Task]) -> Result<Vec<String>, ParError> {
    validate_dag(tasks)?;

    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for task in tasks {
        in_degree.entry(task.id.as_str()).or_insert(0);
        for dep in &task.deps {
            adj.entry(dep.as_str())
                .or_default()
                .push(task.id.as_str());
            *in_degree.entry(task.id.as_str()).or_insert(0) += 1;
        }
    }

    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|&(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut order = Vec::new();
    while let Some(node) = queue.pop_front() {
        order.push(node.to_string());
        if let Some(neighbors) = adj.get(node) {
            for &next in neighbors {
                let deg = in_degree.get_mut(next).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(next);
                }
            }
        }
    }

    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_task_line() {
        let task = parse_task_line("build\t\techo hello").unwrap();
        assert_eq!(task.id, "build");
        assert!(task.deps.is_empty());
        assert_eq!(task.command, "echo hello");
    }

    #[test]
    fn test_parse_task_line_with_deps() {
        let task = parse_task_line("link\tcompile,assemble\tgcc -o out").unwrap();
        assert_eq!(task.id, "link");
        assert_eq!(task.deps, vec!["compile", "assemble"]);
    }

    #[test]
    fn test_parse_task_line_invalid() {
        assert!(parse_task_line("no-tabs").is_err());
    }

    #[test]
    fn test_validate_dag_ok() {
        let tasks = vec![
            Task { id: "a".into(), deps: vec![], command: "echo a".into() },
            Task { id: "b".into(), deps: vec!["a".into()], command: "echo b".into() },
        ];
        assert!(validate_dag(&tasks).is_ok());
    }

    #[test]
    fn test_validate_dag_unknown_dep() {
        let tasks = vec![
            Task { id: "a".into(), deps: vec!["nonexistent".into()], command: "echo a".into() },
        ];
        assert!(matches!(validate_dag(&tasks), Err(ParError::UnknownDep { .. })));
    }

    #[test]
    fn test_validate_dag_cycle() {
        let tasks = vec![
            Task { id: "a".into(), deps: vec!["b".into()], command: "echo a".into() },
            Task { id: "b".into(), deps: vec!["a".into()], command: "echo b".into() },
        ];
        assert!(matches!(validate_dag(&tasks), Err(ParError::CycleDetected(_))));
    }

    #[test]
    fn test_topological_order() {
        let tasks = vec![
            Task { id: "c".into(), deps: vec!["a".into(), "b".into()], command: "".into() },
            Task { id: "a".into(), deps: vec![], command: "".into() },
            Task { id: "b".into(), deps: vec!["a".into()], command: "".into() },
        ];
        let order = topological_order(&tasks).unwrap();
        let pos_a = order.iter().position(|x| x == "a").unwrap();
        let pos_b = order.iter().position(|x| x == "b").unwrap();
        let pos_c = order.iter().position(|x| x == "c").unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn test_execute_simple() {
        let tasks = vec![
            Task { id: "a".into(), deps: vec![], command: "true".into() },
            Task { id: "b".into(), deps: vec!["a".into()], command: "true".into() },
        ];
        let results = execute_par(&tasks, 2, false, 0, false).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.success));
    }

    #[test]
    fn test_execute_failure() {
        let tasks = vec![
            Task { id: "a".into(), deps: vec![], command: "false".into() },
        ];
        let results = execute_par(&tasks, 1, false, 0, false).unwrap();
        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
    }
}
