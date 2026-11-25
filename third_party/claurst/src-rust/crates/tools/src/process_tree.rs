use std::io;
use tokio::process::{Child, Command};

pub(crate) struct ProcessTree {
    #[cfg(windows)]
    job: job::Job,
}

pub(crate) fn spawn_process_tree(command: &mut Command) -> io::Result<(Child, ProcessTree)> {
    command.kill_on_drop(true);
    #[cfg(windows)]
    {
        let job = job::Job::killing_on_close()?;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
        let child = command.spawn()?;
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("spawned process has no handle"))?;
        job.assign(process)?;
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("spawned process has no id"))?;
        job::resume(pid)?;
        Ok((child, ProcessTree { job }))
    }
    #[cfg(not(windows))]
    {
        Ok((command.spawn()?, ProcessTree {}))
    }
}

impl ProcessTree {
    pub(crate) fn release(self) {
        #[cfg(windows)]
        self.job.release();
    }
}

#[cfg(windows)]
mod job {
    use std::io;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_BASIC_LIMIT_INFORMATION,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    pub(super) struct Job(HANDLE);

    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub(super) fn killing_on_close() -> io::Result<Self> {
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Job(handle);
            job.limit(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)?;
            Ok(job)
        }

        pub(super) fn assign(&self, process: HANDLE) -> io::Result<()> {
            if unsafe { AssignProcessToJobObject(self.0, process) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub(super) fn release(self) {
            let _ = self.limit(0);
        }

        fn limit(&self, flags: JOB_OBJECT_LIMIT) -> io::Result<()> {
            let info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
                BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                    LimitFlags: flags,
                    ..Default::default()
                },
                ..Default::default()
            };
            let ok = unsafe {
                SetInformationJobObject(
                    self.0,
                    JobObjectExtendedLimitInformation,
                    &info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION
                        as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub(super) fn resume(pid: u32) -> io::Result<()> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut resumed = 0;
        let mut more = unsafe { Thread32First(snapshot, &mut entry) } != 0;
        while more {
            if entry.th32OwnerProcessID == pid {
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if !thread.is_null() {
                    if unsafe { ResumeThread(thread) } != u32::MAX {
                        resumed += 1;
                    }
                    unsafe {
                        CloseHandle(thread);
                    }
                }
            }
            more = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
        }
        unsafe {
            CloseHandle(snapshot);
        }
        if resumed == 0 {
            return Err(io::Error::other("spawned process could not be resumed"));
        }
        Ok(())
    }
}
