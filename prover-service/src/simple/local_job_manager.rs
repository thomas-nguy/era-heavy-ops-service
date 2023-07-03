use super::*;

use std::{
    io::Write,
    sync::{
        atomic::{AtomicUsize, Ordering},
    },
};
use rand::Rng;

#[derive(Debug)]
pub enum JobState {
    Created(JobId),
    Started(JobId),
    Failure(JobId, String),
    Success(JobId),
}

pub struct LocalJobManager {
    blob_store: Box<dyn ObjectStore>,
    specialized_circuit_ids: Vec<u8>,
}

impl LocalJobManager {
    pub fn new(
        blob_store: Box<dyn ObjectStore>,
        specialized_circuit_ids: Vec<u8>,

    ) -> Self {
        Self { blob_store,  specialized_circuit_ids}
    }

    fn get_job(&mut self, circuit_id: Option<u8>) -> Option<(JobId, ZkSyncCircuit)> {
        None
    }

}

impl JobManager for FullJobManager {
    fn get_next_job(&mut self) -> (JobId, ZkSyncCircuit) {
        loop {
            if let Some(job) = self.get_job(None) {
                return job;
            }
        }
    }

    fn get_next_job_by_circuit(&mut self, circuit_id: u8) -> (JobId, ZkSyncCircuit) {
        loop {
            if let Some(job) = self.get_job(Some(circuit_id)) {
                return job;
            }
        }
    }

    fn try_get_next_job(&mut self) -> Option<(JobId, ZkSyncCircuit)> {
        self.get_job(None)
    }

    fn try_get_next_job_by_circuit(&mut self, circuit_id: u8) -> Option<(JobId, ZkSyncCircuit)> {
        self.get_job(Some(circuit_id))
    }
}

pub struct SimpleJobReporter {
    jobs: Arc<Mutex<Vec<(usize, ZkSyncCircuit, JobState)>>>,
    next_job_id: AtomicUsize,
}

impl SimpleJobReporter {
    pub fn new(jobs: Arc<Mutex<Vec<(usize, ZkSyncCircuit, JobState)>>>) -> Self {
        Self {
            jobs,
            next_job_id: AtomicUsize::new(0),
        }
    }
}

impl JobReporter for SimpleJobReporter {
    fn send_report(&mut self, report: JobResult) {
        println!("{:?}", &report);
        match &report {
            JobResult::ProverWaitedIdle(prover_idx, duration) => {
                println!("prover {} waited {:?}", prover_idx, duration);
                return;
            }
            JobResult::SetupLoaderWaitedIdle(duration) => {
                println!("setup loader waited {:?}", duration);
                return;
            }
            JobResult::SchedulerWaitedIdle(duration) => {
                println!("job scheduler waited {:?}", duration);
                return;
            }
            _ => (),
        }

        let job_id = match report.clone() {
            JobResult::Synthesized(job_id, _)
            | JobResult::AssemblyFinalized(job_id, _)
            | JobResult::SetupLoaded(job_id, _, _)
            | JobResult::ProofGenerated(job_id, _, _, _)
            | JobResult::Failure(job_id, _) => job_id,
            JobResult::AssemblyEncoded(job_id, _) => job_id,
            JobResult::AssemblyDecoded(job_id, _) => job_id,
            JobResult::AssemblyTransferred(job_id, _) => job_id,
            JobResult::FailureWithDebugging(job_id, _, _, _) => job_id,
            _ => unreachable!(),
        };

        let mut this_job = None;

        let mut jobs = self.jobs.lock().unwrap();
        rand::thread_rng().shuffle(&mut jobs);
        for job in jobs.iter_mut() {
            match job {
                (inner_job_id, _, _) => {
                    if *inner_job_id == job_id {
                        this_job = Some(job);
                    }
                }
                _ => (),
            }
        }

        if let Some(job) = this_job {
            match &report {
                JobResult::AssemblyTransferred(job_id, _) => {
                    let new_job_id = self.next_job_id.fetch_add(1, Ordering::SeqCst);
                    job.0 = new_job_id;
                    job.2 = JobState::Created(new_job_id);
                }
                JobResult::ProofGenerated(_, _, _, _) => {
                    let new_job_id = self.next_job_id.fetch_add(1, Ordering::SeqCst);
                    job.0 = new_job_id;
                    job.2 = JobState::Created(new_job_id);
                }
                JobResult::Failure(_, msg) => {
                    job.2 = JobState::Failure(job_id, msg.clone());
                }
                JobResult::FailureWithDebugging(_, _, _, msg) => {
                    job.2 = JobState::Failure(job_id, msg.clone());
                }
                _ => (),
            }
        }
        rand::thread_rng().shuffle(&mut jobs);
        handle_report(&report, job_id);
    }
}

fn handle_report(report: &JobResult, job_id: usize) {
    match report {
        JobResult::Synthesized(_, duration) => {
            append_into_file("synthesized.log", &format!("{}\t{:?}", job_id, duration));
        }
        JobResult::ProverWaitedIdle(_, duration) => {
            // append_into_file("synthesized.log", &format!("{}\t{:?}", job_id, duration));
        }
        JobResult::AssemblyFinalized(_, duration) => {
            append_into_file(
                "assembly_finalized.log",
                &format!("{}\t{:?}", job_id, duration),
            );
        }
        JobResult::AssemblyEncoded(_, duration) => {
            append_into_file(
                "assembly_encoded.log",
                &format!("{}\t{:?}", job_id, duration),
            );
        }
        JobResult::AssemblyDecoded(_, duration) => {
            append_into_file(
                "assembly_decoded.log",
                &format!("{}\t{:?}", job_id, duration),
            );
        }
        JobResult::AssemblyTransferred(_, duration) => {
            append_into_file(
                "assembly_transferred.log",
                &format!("{}\t{:?}", job_id, duration),
            );
        }
        JobResult::SetupLoaded(_, duration, _) => {
            append_into_file("setup_loaded.log", &format!("{}\t{:?}", job_id, duration));
        }
        JobResult::ProofGenerated(_, duration, _, prover_idx) => {
            // reuse successfull jobs
            append_into_file(
                "proof_generated.log",
                &format!("{} {}\t{:?}", prover_idx, job_id, duration),
            );
        }
        JobResult::Failure(_, ref duration) => {
            append_into_file("failure.log", &format!("{}\t{:?}", job_id, duration));
        }
        JobResult::FailureWithDebugging(job_id, circuit_id, ref assembly_encoding, ref msg) => {
            let artifacts_dir = get_artifacts_dir();
            let artifacts_dir = artifacts_dir.to_string_lossy().to_string();
            let assembly_file_path = format!(
                "{}/failed_assembly_encoding_{}_{}.bin",
                artifacts_dir, circuit_id, job_id
            );
            let mut assembly_file = std::fs::File::create(assembly_file_path).unwrap();
            assembly_file.write_all(&assembly_encoding).unwrap();

            append_into_file(
                "assembly_decoding_failure.log",
                &format!("{}\t{}", job_id, circuit_id),
            );
        }
        _ => unreachable!(),
    }
}
fn append_into_file(path: &str, content: &str) {
    if !std::path::Path::new(&format!("/tmp/{}", path)).exists() {
        std::fs::File::create(&format!("/tmp/{}", path)).expect("Unable to create file");
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(format!("/tmp/{}", path))
        .expect("Unable to open file");
    if let Err(e) = writeln!(file, "{}", content) {
        eprintln!("Couldn't write to file: {}", e);
    }
}
