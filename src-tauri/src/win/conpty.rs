//! Runs one console program in a hidden pseudo console (ConPTY) and captures
//! its output. The child is placed in a Job Object with KILL_ON_JOB_CLOSE, so
//! it (and anything it starts) cannot outlive the call.
//!
//! Droplet uses this for exactly one program: agy.exe.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::Console::{ClosePseudoConsole, CreatePseudoConsole, COORD, HPCON};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, ResumeThread, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_SUSPENDED, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, STARTUPINFOEXW,
};

use super::wide;

/// A handle that closes itself.
struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}
// The handle is only read by the capture thread.
struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

/// Runs `exe` with `args` and returns what it printed (at most `max_bytes`),
/// or `None` if it could not be started. The process is killed after `timeout`.
pub fn run_hidden(exe: &Path, args: &[&str], timeout: Duration, max_bytes: usize) -> Option<String> {
    unsafe {
        // Pipes: we write nothing to the child's input and read its output.
        let (mut in_read, mut in_write, mut out_read, mut out_write) = (HANDLE::default(), HANDLE::default(), HANDLE::default(), HANDLE::default());
        CreatePipe(&mut in_read, &mut in_write, None, 0).ok()?;
        let (in_read, _in_write) = (Owned(in_read), Owned(in_write));
        CreatePipe(&mut out_read, &mut out_write, None, 0).ok()?;
        let (out_read, out_write) = (Owned(out_read), Owned(out_write));

        let console: HPCON = CreatePseudoConsole(COORD { X: 160, Y: 50 }, in_read.0, out_write.0, 0).ok()?;
        // The pseudo console holds its own copies of these ends now.
        drop(out_write);
        drop(in_read);

        // Job object: everything in it dies when the job handle closes.
        let job = Owned(CreateJobObjectW(None, PCWSTR::null()).ok()?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let _ = SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );

        // Attach the pseudo console to the new process.
        let mut size = 0usize;
        let _ = InitializeProcThreadAttributeList(None, 1, None, &mut size);
        let mut attributes = vec![0u8; size];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(attributes.as_mut_ptr() as *mut _);
        if InitializeProcThreadAttributeList(Some(list), 1, None, &mut size).is_err() {
            ClosePseudoConsole(console);
            return None;
        }
        let attached = UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            Some(console.0 as *const _),
            std::mem::size_of::<HPCON>(),
            None,
            None,
        );
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = list;

        // The executable is passed explicitly, so nothing is searched on PATH and no shell is involved.
        let mut command = wide(&std::iter::once(format!("\"{}\"", exe.display())).chain(args.iter().map(|a| a.to_string())).collect::<Vec<_>>().join(" "));
        let application = wide(&exe.to_string_lossy());
        let mut process = PROCESS_INFORMATION::default();
        let started = attached.is_ok()
            && CreateProcessW(
                PCWSTR(application.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                false,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED,
                None,
                None,
                &startup.StartupInfo,
                &mut process,
            )
            .is_ok();
        DeleteProcThreadAttributeList(list);
        if !started {
            ClosePseudoConsole(console);
            return None;
        }
        let (process_handle, thread_handle) = (Owned(process.hProcess), Owned(process.hThread));
        let _ = AssignProcessToJobObject(job.0, process_handle.0);
        ResumeThread(thread_handle.0);

        // Capture output on a helper thread; ReadFile ends when the pseudo console closes.
        let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
        let (sink, pipe) = (captured.clone(), SendHandle(out_read.0));
        let reader = std::thread::spawn(move || {
            let pipe = pipe;
            let mut chunk = [0u8; 4096];
            loop {
                let mut read = 0u32;
                if ReadFile(pipe.0, Some(&mut chunk), Some(&mut read), None).is_err() || read == 0 {
                    break;
                }
                let mut buffer = sink.lock().unwrap();
                let room = max_bytes.saturating_sub(buffer.len());
                buffer.extend_from_slice(&chunk[..(read as usize).min(room)]);
            }
        });

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if WaitForSingleObject(process_handle.0, 200) == WAIT_OBJECT_0 {
                break;
            }
            if captured.lock().unwrap().len() >= max_bytes {
                break;
            }
        }
        // Closing the job kills the child if it is still running; closing the
        // pseudo console flushes and ends the output pipe so the reader stops.
        drop(job);
        ClosePseudoConsole(console);
        let _ = reader.join();
        drop(out_read);
        let bytes = std::mem::take(&mut *captured.lock().unwrap());
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}
