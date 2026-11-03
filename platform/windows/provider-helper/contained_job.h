#pragma once
#include <windows.h>
#include <cstdio>
#include <exception>
#include <stdexcept>
#include <string>
inline bool wait_owned_job_empty(HANDLE job,DWORD timeout) noexcept {
    const auto deadline=GetTickCount64()+timeout;
    do {
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION counters{};
        if(!QueryInformationJobObject(job,JobObjectBasicAccountingInformation,&counters,sizeof(counters),nullptr))return false;
        if(!counters.ActiveProcesses)return true;
        Sleep(10);
    } while(GetTickCount64()<deadline);
    return false;
}
// Reserved exit contract for the pinned helper's RUN mode only. A provider can
// choose its own status, but cannot turn an unproved cleanup into helper0/125.
inline int settled_scope_exit_code(DWORD worker_exit, bool provider_settled, bool proxy_settled) {
    if (!provider_settled || !proxy_settled) throw std::runtime_error("owned-scope-settlement-unproven");
    return worker_exit == 0 ? 0 : 125;
}
struct OwnedJobLifetime {
    HANDLE job;
    ~OwnedJobLifetime() noexcept {
        if (!TerminateJobObject(job, 126)) { std::fputs("GENT-SANDBOX-ERROR job-stop-unproven\n", stderr); std::terminate(); }
        const auto deadline = GetTickCount64() + 5000;
        do {
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION counters{};
            if (!QueryInformationJobObject(job, JobObjectBasicAccountingInformation, &counters, sizeof(counters), nullptr)) break;
            if (!counters.ActiveProcesses) return;
            Sleep(10);
        } while (GetTickCount64() < deadline);
        std::fputs("GENT-SANDBOX-ERROR job-settlement-unproven\n", stderr); std::terminate();
    }
};

inline std::string job_accounting_text(HANDLE job,const char* label) {
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION counters{};
    if(!QueryInformationJobObject(job,JobObjectBasicAccountingInformation,&counters,sizeof(counters),nullptr))throw std::runtime_error("job-accounting");
    return std::string(label)+"_active="+std::to_string(counters.ActiveProcesses)+" "+label+"_total="+std::to_string(counters.TotalProcesses);
}

inline void bind_suspended_to_job(HANDLE process, HANDLE thread, HANDLE job) {
    auto check = [](bool ok, const char* label) {
        if (!ok) { const auto error = GetLastError(); throw std::runtime_error(std::string(label) + ":" + std::to_string(error)); }
    };
    check(GetProcessIdOfThread(thread) == GetProcessId(process), "suspended-job-thread-binding");
    const auto previous = SuspendThread(thread);
    check(previous != static_cast<DWORD>(-1), "suspended-job-thread-query");
    check(previous > 0, "suspended-job-was-running");
    check(ResumeThread(thread) == previous + 1, "suspended-job-count-restore");
    BOOL member = FALSE;
    check(IsProcessInJob(process, job, &member) != FALSE, "suspended-job-query");
    if (!member) check(AssignProcessToJobObject(job, process) != FALSE, "suspended-job-assign");
    check(IsProcessInJob(process, job, &member) && member, "suspended-job-membership");
}
