use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unknown engine: {0}")]
    UnknownEngine(String),
    #[error("job not found: {0}")]
    JobNotFound(String),
    #[error("engine command failed: {0}")]
    CommandFailed(String),
    #[error("unsupported operation for engine: {0}")]
    Unsupported(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Pending => write!(f, "pending"),
            JobStatus::Running => write!(f, "running"),
            JobStatus::Done => write!(f, "done"),
            JobStatus::Failed => write!(f, "failed"),
        }
    }
}

/// Job info returned by list/status.
#[derive(Debug, Clone)]
pub struct JobInfo {
    pub id: String,
    pub status: JobStatus,
    pub command: String,
}

/// Trait for queue engines.
pub trait QueueEngine: Send + Sync {
    fn name(&self) -> &str;
    fn submit(&self, command: &str, deps: &[String], args: &SubmitArgs) -> Result<String, QueueError>;
    fn status(&self, job_id: &str) -> Result<JobStatus, QueueError>;
    fn wait(&self, job_ids: &[String]) -> Result<Vec<(String, JobStatus)>, QueueError>;
    fn cancel(&self, job_id: &str) -> Result<(), QueueError>;
    fn list(&self) -> Result<Vec<JobInfo>, QueueError>;
}

/// Extra arguments for job submission.
#[derive(Debug, Clone, Default)]
pub struct SubmitArgs {
    pub slots: Option<usize>,
    pub mem: Option<String>,
    pub time: Option<String>,
    pub extra: Vec<String>,
}

// === Local engine ===

pub struct LocalEngine {
    next_id: Arc<Mutex<u64>>,
    jobs: Arc<Mutex<HashMap<String, LocalJob>>>,
}

struct LocalJob {
    command: String,
    handle: Option<thread::JoinHandle<i32>>,
    status: Arc<Mutex<JobStatus>>,
}

impl LocalEngine {
    pub fn new() -> Self {
        Self {
            next_id: Arc::new(Mutex::new(1)),
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for LocalEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl QueueEngine for LocalEngine {
    fn name(&self) -> &str {
        "local"
    }

    fn submit(&self, command: &str, _deps: &[String], _args: &SubmitArgs) -> Result<String, QueueError> {
        let mut next_id = self.next_id.lock().unwrap();
        let id = format!("local-{}", *next_id);
        *next_id += 1;

        let status = Arc::new(Mutex::new(JobStatus::Running));
        let status_clone = Arc::clone(&status);
        let cmd = command.to_string();

        let handle = thread::spawn(move || {
            let result = Command::new("sh").arg("-c").arg(&cmd).status();
            let exit = match result {
                Ok(s) => s.code().unwrap_or(1),
                Err(_) => 127,
            };
            let mut s = status_clone.lock().unwrap();
            *s = if exit == 0 {
                JobStatus::Done
            } else {
                JobStatus::Failed
            };
            exit
        });

        self.jobs.lock().unwrap().insert(
            id.clone(),
            LocalJob {
                command: command.to_string(),
                handle: Some(handle),
                status,
            },
        );

        Ok(id)
    }

    fn status(&self, job_id: &str) -> Result<JobStatus, QueueError> {
        let jobs = self.jobs.lock().unwrap();
        match jobs.get(job_id) {
            Some(job) => Ok(*job.status.lock().unwrap()),
            None => Err(QueueError::JobNotFound(job_id.into())),
        }
    }

    fn wait(&self, job_ids: &[String]) -> Result<Vec<(String, JobStatus)>, QueueError> {
        let mut results = Vec::new();
        for id in job_ids {
            let handle = {
                let mut jobs = self.jobs.lock().unwrap();
                match jobs.get_mut(id) {
                    Some(job) => job.handle.take(),
                    None => return Err(QueueError::JobNotFound(id.clone())),
                }
            };
            if let Some(h) = handle {
                let _ = h.join();
            }
            let status = self.status(id)?;
            results.push((id.clone(), status));
        }
        Ok(results)
    }

    fn cancel(&self, job_id: &str) -> Result<(), QueueError> {
        let jobs = self.jobs.lock().unwrap();
        if jobs.contains_key(job_id) {
            // Local jobs can't be easily cancelled; mark as done
            Ok(())
        } else {
            Err(QueueError::JobNotFound(job_id.into()))
        }
    }

    fn list(&self) -> Result<Vec<JobInfo>, QueueError> {
        let jobs = self.jobs.lock().unwrap();
        let mut infos = Vec::new();
        for (id, job) in jobs.iter() {
            infos.push(JobInfo {
                id: id.clone(),
                status: *job.status.lock().unwrap(),
                command: job.command.clone(),
            });
        }
        Ok(infos)
    }
}

// === SLURM engine ===

#[cfg(feature = "slurm")]
pub struct SlurmEngine;

#[cfg(feature = "slurm")]
impl QueueEngine for SlurmEngine {
    fn name(&self) -> &str { "slurm" }

    fn submit(&self, command: &str, deps: &[String], args: &SubmitArgs) -> Result<String, QueueError> {
        let mut cmd = Command::new("sbatch");
        cmd.arg("--parsable");

        if let Some(slots) = args.slots {
            cmd.arg(format!("--ntasks={slots}"));
        }
        if let Some(ref mem) = args.mem {
            cmd.arg(format!("--mem={mem}"));
        }
        if let Some(ref time) = args.time {
            cmd.arg(format!("--time={time}"));
        }
        if !deps.is_empty() {
            cmd.arg(format!("--dependency=afterok:{}", deps.join(":")));
        }
        for extra in &args.extra {
            cmd.arg(extra);
        }
        cmd.arg("--wrap").arg(command);

        let output = cmd.output()?;
        if !output.status.success() {
            return Err(QueueError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn status(&self, job_id: &str) -> Result<JobStatus, QueueError> {
        let output = Command::new("squeue")
            .args(["--job", job_id, "--noheader", "-o", "%T"])
            .output()?;
        let state = String::from_utf8_lossy(&output.stdout).trim().to_uppercase();
        Ok(match state.as_str() {
            "PENDING" => JobStatus::Pending,
            "RUNNING" => JobStatus::Running,
            "COMPLETED" => JobStatus::Done,
            "" => JobStatus::Done, // No longer in queue
            _ => JobStatus::Failed,
        })
    }

    fn wait(&self, job_ids: &[String]) -> Result<Vec<(String, JobStatus)>, QueueError> {
        // Use srun --dependency to wait
        for id in job_ids {
            let _ = Command::new("srun")
                .args(["--dependency", &format!("afterany:{id}"), "true"])
                .status();
        }
        job_ids.iter().map(|id| {
            let status = self.status(id)?;
            Ok((id.clone(), status))
        }).collect()
    }

    fn cancel(&self, job_id: &str) -> Result<(), QueueError> {
        let status = Command::new("scancel").arg(job_id).status()?;
        if status.success() { Ok(()) } else { Err(QueueError::CommandFailed("scancel failed".into())) }
    }

    fn list(&self) -> Result<Vec<JobInfo>, QueueError> {
        let output = Command::new("squeue")
            .args(["--me", "--noheader", "-o", "%i\t%T\t%o"])
            .output()?;
        let mut jobs = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() >= 3 {
                let status = match parts[1].to_uppercase().as_str() {
                    "PENDING" => JobStatus::Pending,
                    "RUNNING" => JobStatus::Running,
                    "COMPLETED" => JobStatus::Done,
                    _ => JobStatus::Failed,
                };
                jobs.push(JobInfo { id: parts[0].into(), status, command: parts[2].into() });
            }
        }
        Ok(jobs)
    }
}

/// Create an engine by name.
pub fn create_engine(name: &str) -> Result<Box<dyn QueueEngine>, QueueError> {
    match name {
        "local" => Ok(Box::new(LocalEngine::new())),
        #[cfg(feature = "slurm")]
        "slurm" => Ok(Box::new(SlurmEngine)),
        other => Err(QueueError::UnknownEngine(other.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_submit_and_wait() {
        let engine = LocalEngine::new();
        let id = engine
            .submit("true", &[], &SubmitArgs::default())
            .unwrap();
        assert!(id.starts_with("local-"));

        let results = engine.wait(&[id.clone()]).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, JobStatus::Done);
    }

    #[test]
    fn test_local_submit_failure() {
        let engine = LocalEngine::new();
        let id = engine
            .submit("false", &[], &SubmitArgs::default())
            .unwrap();
        let results = engine.wait(&[id]).unwrap();
        assert_eq!(results[0].1, JobStatus::Failed);
    }

    #[test]
    fn test_local_list() {
        let engine = LocalEngine::new();
        let id = engine
            .submit("sleep 0.01", &[], &SubmitArgs::default())
            .unwrap();
        let jobs = engine.list().unwrap();
        assert!(jobs.iter().any(|j| j.id == id));
    }

    #[test]
    fn test_local_status_not_found() {
        let engine = LocalEngine::new();
        assert!(matches!(
            engine.status("nonexistent"),
            Err(QueueError::JobNotFound(_))
        ));
    }

    #[test]
    fn test_create_engine() {
        let engine = create_engine("local").unwrap();
        assert_eq!(engine.name(), "local");
        assert!(create_engine("nonexistent").is_err());
    }
}
