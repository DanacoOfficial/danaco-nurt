// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Cancellation of a call while the detached supervisor starts its run.

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

/// Process control double whose start waits for a gate file and records that
/// the start began.
struct GatedStart {
    inner: FakeControl,
    gate: PathBuf,
    entered: Arc<AtomicBool>,
}

impl ProcessControl for GatedStart {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Arc<dyn OutputSink>,
        cancel: CancelToken,
    ) -> Result<Box<dyn RunningProcess>> {
        self.entered.store(true, Ordering::SeqCst);
        wait_until(|| self.gate.exists());
        self.inner.spawn(request, sink, cancel)
    }
}

#[test]
fn a_cancelled_call_stops_a_run_the_supervisor_already_took() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    let gate = path.join("gate");
    let entered = Arc::new(AtomicBool::new(false));
    let components = Components {
        control: Arc::new(GatedStart {
            inner: FakeControl::default(),
            gate: gate.clone(),
            entered: Arc::clone(&entered),
        }),
        spool: Arc::new(FileSpool),
        tracker: Arc::new(FakeTracker),
    };
    let (server, _) = context(&path);
    let _serving = {
        let components = components.clone();
        std::thread::spawn(move || supervisor::serve(&server, &components))
    };
    let table = JobTable::at(path.join("jobs")).unwrap();
    wait_until(|| table.host_running().unwrap());
    let command = SupervisorCommand {
        program: path.join("nurt"),
        args: Vec::new(),
    };
    let (client, _) = context_with(&path, Config::default(), Some(Supervision::Detached(command)));
    let cancel = client.cancel().clone();
    let host = run_host(&client, components);
    let caller = {
        let path = path.clone();
        std::thread::spawn(move || {
            host.run(
                &client,
                request(&path, 10_000, 0, 0),
                Some(Duration::from_secs(30)),
            )
        })
    };
    wait_until(|| entered.load(Ordering::SeqCst));
    cancel.cancel(CancelReason::Requested);
    // The caller polls every 50 ms; it has seen the cancellation long before
    // the supervisor answers.
    std::thread::sleep(control::CONTROL_POLL * 10);
    fs::write(&gate, b"").unwrap();
    let RunWait::Finished(result) = caller.join().unwrap().unwrap() else {
        panic!("the run moved to the background");
    };
    assert_eq!(result.end.killed_by, Some(KilledBy::Cancel));
    let id = table.ids().unwrap().pop().unwrap();
    assert_eq!(job_file(&path, &id).record.state, JobState::Stopped);
}
