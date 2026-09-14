//! Windows: one job object per child, `CREATE_NO_WINDOW`.
//!
//! Each spawned child gets its own job object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Grandchildren inherit the job, so
//! terminating the job kills the whole tree, and when the app dies (even via
//! Task Manager) every job handle closes and every tree dies with it.
//!
//! Known gap: a grandchild spawned in the few microseconds between `spawn` and
//! `AssignProcessToJobObject` would escape the job. Compilers and language
//! servers don't fork that early; accepted.

use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use tracing::warn;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SET_QUOTA, PROCESS_TERMINATE, TerminateProcess,
};

/// `STILL_ACTIVE` as returned by `GetExitCodeProcess`.
const STILL_ACTIVE: u32 = 259;

/// Holds the job object for one child. Dropping it closes the handle, which
/// terminates the job because of `KILL_ON_JOB_CLOSE`.
pub struct Guard {
    job: HANDLE,
}

// HANDLE is a raw pointer; the job object is safe to use from any thread.
unsafe impl Send for Guard {}
unsafe impl Sync for Guard {}

impl Drop for Guard {
    fn drop(&mut self) {
        if !self.job.is_null() {
            // SAFETY: `job` is a valid handle we own.
            unsafe { CloseHandle(self.job) };
        }
    }
}

/// Hide the console window that console programs would otherwise flash.
pub fn prepare(cmd: &mut tokio::process::Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// Create a job object and assign the child to it.
pub fn register_child(pid: u32, program: &str) -> Guard {
    // SAFETY: plain Win32 calls with valid arguments; handles are closed on
    // every path.
    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            warn!(
                pid,
                program, "CreateJobObjectW failed; child will not be tree-killed"
            );
            return Guard { job: null_mut() };
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            warn!(pid, program, "SetInformationJobObject failed");
        }

        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if process.is_null() {
            warn!(
                pid,
                program, "OpenProcess failed; child not assigned to job"
            );
        } else {
            if AssignProcessToJobObject(job, process) == 0 {
                warn!(pid, program, "AssignProcessToJobObject failed");
            }
            CloseHandle(process);
        }

        Guard { job }
    }
}

/// Terminate the child's whole tree. Windows has no "polite" termination for
/// console processes without a window, so this is the same as the hard path.
pub fn terminate(guard: &Guard, pid: u32) {
    terminate_hard(guard, pid);
}

/// Terminate the job (whole tree). Falls back to the single process if the job
/// could not be created.
pub fn terminate_hard(guard: &Guard, pid: u32) {
    // SAFETY: valid handles; exit code 1 is arbitrary.
    unsafe {
        if !guard.job.is_null() {
            TerminateJobObject(guard.job, 1);
            return;
        }
        let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !process.is_null() {
            TerminateProcess(process, 1);
            CloseHandle(process);
        }
    }
}

/// Whether a process with this pid is still running.
pub fn is_alive(pid: u32) -> bool {
    // SAFETY: handle is closed before returning.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(process, &mut code);
        CloseHandle(process);
        ok != 0 && code == STILL_ACTIVE
    }
}

/// No pid registry is needed on Windows; job objects cover force-quit.
pub fn set_registry(_path: PathBuf) {}

/// Nothing to sweep on Windows.
pub fn sweep_orphans() -> usize {
    0
}
