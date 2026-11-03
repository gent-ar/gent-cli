#include "readonly_bootstrap.h"
#include <windows.h>
#include <bcrypt.h>
#include <shlobj.h>
#include <sddl.h>
#include <algorithm>
#include <array>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>
#include "readonly_authority.h"
#include "gateway_proxy.h" // Historical owned-profile cleanup only.
#include "external_package_proxy.h"
#include "external-package/package_authority.h"
#include "contained_job.h"
#include "control_scope.h"
#include "provider_capabilities.h"
#include "psec_environment.h"
#include "command_broker.h"
#include "filesystem_aliases.h"
#include "resource_inspection.h"

struct Handle {
    HANDLE h = INVALID_HANDLE_VALUE;
    Handle() = default;
    explicit Handle(HANDLE value) : h(value) {}
    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;
    Handle(Handle&& other) noexcept : h(other.h) { other.h = INVALID_HANDLE_VALUE; }
    void close() { if (h && h != INVALID_HANDLE_VALUE) CloseHandle(h); h = INVALID_HANDLE_VALUE; }
    ~Handle() { close(); }
};
[[noreturn]] static void fail(const char* label) { const auto error=GetLastError(); throw std::runtime_error(std::string(label) + ":" + std::to_string(error)); }
static void check(bool ok, const char* label) { if (!ok) fail(label); }
static void hresult(HRESULT hr, const char* label) { if (FAILED(hr)) throw std::runtime_error(std::string(label) + ":" + std::to_string(static_cast<uint32_t>(hr))); }
using gent_psec::utf8;
static std::wstring quote(const std::wstring& value) {
    std::wstring result = L"\""; size_t slash = 0;
    for (wchar_t c : value) { if (c == L'\\') { ++slash; continue; }
        result.append(c == L'"' ? slash * 2 + 1 : slash, L'\\'); slash = 0; result += c; }
    result.append(slash * 2, L'\\'); return result + L'"';
}
static bool digest_string(const std::wstring& value) {
    return value.size() == 64 && std::all_of(value.begin(), value.end(), [](wchar_t c) { return (c >= L'0' && c <= L'9') || (c >= L'a' && c <= L'f'); });
}
using gent_psec::normalized_path;
static std::wstring hash(HANDLE file) {
    BCRYPT_ALG_HANDLE algorithm = nullptr; BCRYPT_HASH_HANDLE state = nullptr;
    check(BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) >= 0, "hash-open"); DWORD size = 0, read = 0;
    check(BCryptGetProperty(algorithm, BCRYPT_OBJECT_LENGTH, reinterpret_cast<PUCHAR>(&size), sizeof(size), &read, 0) >= 0, "hash-size");
    std::vector<UCHAR> object(size); std::array<UCHAR, 32> digest{};
    check(BCryptCreateHash(algorithm, &state, object.data(), size, nullptr, 0, 0) >= 0, "hash-create"); std::array<UCHAR, 65536> bytes{};
    do { check(ReadFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &read, nullptr) != 0, "hash-read");
        if (read) check(BCryptHashData(state, bytes.data(), read, 0) >= 0, "hash-update"); } while (read);
    check(BCryptFinishHash(state, digest.data(), static_cast<ULONG>(digest.size()), 0) >= 0, "hash-finish"); BCryptDestroyHash(state); BCryptCloseAlgorithmProvider(algorithm, 0);
    std::wstring result; for (auto b : digest) { result += L"0123456789abcdef"[b >> 4]; result += L"0123456789abcdef"[b & 15]; } return result;
}
static Handle directory_lock(const std::wstring& path) {
    check(path.size() > 3 && (path[1] == L':' || path.rfind(L"\\\\?\\", 0) == 0), "absolute-root");
    size_t start = path.rfind(L"\\\\?\\", 0) == 0 ? 7 : 3;
    for (size_t i = start; i <= path.size(); ++i) { if (i != path.size() && path[i] != L'\\' && path[i] != L'/') continue;
        auto component = path.substr(start, i - start); check(component != L"." && component != L"..", "root-components"); start = i + 1;
        DWORD attr = GetFileAttributesW(path.substr(0, i).c_str()); check(attr != INVALID_FILE_ATTRIBUTES && !(attr & FILE_ATTRIBUTE_REPARSE_POINT), "reparse-root"); }
    Handle result(CreateFileW(path.c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, nullptr));
    check(result.h != INVALID_HANDLE_VALUE, "directory-lock"); return result;
}
#include "credential_probe.h"
#include "probe_unlink_handle.h"
#include "launch_options.h"
static gent_psec::Policy provider_policy(const Options& o) {
    gent_psec::Policy policy; policy.read = o.read; policy.write = o.write;
    policy.network = o.gateway_port ? gent_psec::Network::ProviderGateway : gent_psec::Network::Disabled;
    policy.gateway_peer = o.gateway_peer; policy.proxy_port = o.proxy_port; return policy;
}
// Read-only snapshot: no environment creation, activation, Job, or network access.
// Launch independently revalidates and retains PackageAuthority locks.
static int inspect_external_package(int argc, wchar_t** argv) {
    check(argc == 3, "inspect-external-package-options");
    const std::wstring nonce(argv[2]);
    check(nonce.size() == 32 && std::all_of(nonce.begin(), nonce.end(), [](wchar_t c) {
        return (c >= L'0' && c <= L'9') || (c >= L'a' && c <= L'f');
    }), "inspect-external-package-nonce");
    external_bridge::PackageAuthority package;
    gent_psec::Api api; BOOLEAN available = FALSE; DWORD minor = 0; uint64_t flags = 0;
    hresult(api.version(1, &available, &minor), "inspect-psec-version-query");
    check(available != FALSE, "inspect-psec-version-unavailable");
    hresult(api.query(&flags), "inspect-psec-support-query");
    // Family/full-name/application are restricted ASCII by PackageAuthority;
    // digests are verified lowercase SHA256 and nonce is restricted above.
    const std::string receipt = "{\"schema\":1,\"nonce\":\"" + utf8(nonce)
        + "\",\"package\":{\"family\":\"" + utf8(package.family)
        + "\",\"fullName\":\"" + utf8(package.full_name)
        + "\",\"application\":\"" + utf8(package.application)
        + "\",\"imageSha256\":\"" + package.image_digest
        + "\",\"manifestSha256\":\"" + package.manifest_digest
        + "\",\"developmentRegistration\":" + (package.development_registration ? "true" : "false")
        + "},\"psec\":{\"major\":1,\"minor\":" + std::to_string(minor)
        + ",\"supportFlags\":" + std::to_string(flags) + "}}\n";
    check(receipt.size() <= 4096, "inspect-external-package-receipt-size");
    std::cout << receipt; return 0;
}

#include "launch_environment.h"
static void write_all(HANDLE output, const char* data, size_t size) {
    while (size) { DWORD written = 0; check(WriteFile(output, data, static_cast<DWORD>(std::min<size_t>(size, 65536)), &written, nullptr) && written, "stdio-write"); data += written; size -= written; }
}
static void relay(HANDLE from, HANDLE to) { std::array<char, 32768> data{}; DWORD read = 0;
    while (ReadFile(from, data.data(), static_cast<DWORD>(data.size()), &read, nullptr) && read) { try { write_all(to, data.data(), read); } catch (...) { break; } } }
#include "contained_temp_bootstrap.h"
#include "command_shims.h"
static int run(Options o) {
    wchar_t self_buffer[32768]{};DWORD self_len=GetModuleFileNameW(nullptr,self_buffer,32768);check(self_len && self_len<32768,"bootstrap-self-path");
    std::wstring self(self_buffer,self_len); Handle self_image;
    if(!o.state_root.empty() && !o.shell.empty()) {
        self_image.h=CreateFileW(self.c_str(),GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_SEQUENTIAL_SCAN,nullptr);
        check(self_image.h!=INVALID_HANDLE_VALUE,"bootstrap-self-lock");
        if(std::none_of(o.read.begin(),o.read.end(),[&](const auto& p){return _wcsicmp(normalized_path(p).c_str(),normalized_path(self).c_str())==0;})) o.read.push_back(self);
    }

    gent_psec::Api api;
    std::vector<Handle> directories; directories.push_back(directory_lock(o.workspace)); for (const auto& path : o.read) directories.push_back(directory_lock(path)); for (const auto& path : o.write) directories.push_back(directory_lock(path));
    validate_filesystem_aliases(o.read, o.write);
    uint64_t support = 0; hresult(api.query(&support), "psec-support"); check((support & 1) != 0, "psec-deny-unavailable");
    Handle job(CreateJobObjectW(nullptr, nullptr)); check(job.h != nullptr, "job"); JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{};
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_JOB_MEMORY | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION | (o.cpu_uncapped ? 0 : JOB_OBJECT_LIMIT_JOB_TIME);
    const uint64_t job_share = o.gateway_port ? 2 : 1;
    limits.BasicLimitInformation.ActiveProcessLimit = std::max<DWORD>(1, static_cast<DWORD>(o.processes)); limits.BasicLimitInformation.PerJobUserTimeLimit.QuadPart = static_cast<LONGLONG>(o.cpu / job_share * 10000); limits.JobMemoryLimit = static_cast<SIZE_T>(o.memory / job_share);
    check(SetInformationJobObject(job.h, JobObjectExtendedLimitInformation, &limits, sizeof(limits)) != 0, "job-limits");
    std::unique_ptr<external_bridge::ExternalPackageProxy> gateway;
    if (o.gateway_port) { BOOLEAN available = FALSE; DWORD minor = 0;
        hresult(api.version(1, &available, &minor), "gateway-psec-version"); hresult(api.query(&support), "gateway-psec-support"); check(available && minor >= 1 && (support & 8), "gateway-psec-unavailable"); }
    if (o.gateway_port) {
        gateway = std::make_unique<external_bridge::ExternalPackageProxy>(o.gateway_port, o.state_root, o.peer_nonce, utf8(o.gateway_token));
        // Compare physical handle paths, not lexical aliases/short names. All grant
        // ancestors and the native control namespace remain deny-delete locked.
        auto control_lock=directory_lock(gateway->control_root());
        const auto control=scope_final_path(control_lock.h);
        auto verify_control=[&](const std::wstring& grant){auto handle=directory_lock(grant);check(control_scope_disjoint(control,scope_final_path(handle.h)),"external-control-overlaps-provider-grant");};
        verify_control(o.workspace);for(const auto& grant:o.write)verify_control(grant);
        const auto& run_bearer=gateway->run_bearer();
        o.gateway_token=std::wstring(run_bearer.begin(),run_bearer.end());
        check(digest_string(o.gateway_token),"external-run-bearer-shape");
        o.gateway_peer = gateway->family(); o.proxy_port = gateway->port();
        check(!o.gateway_peer.empty() && !gateway->full_name().empty() && o.proxy_port &&
            WaitForSingleObject(gateway->process(),0)==WAIT_TIMEOUT,"external-package-not-ready");
    }
    gent_psec::Environment sandbox(api, gent_psec::Spec(provider_policy(o))); OwnedJobLifetime job_lifetime{job.h}; std::unique_ptr<CommandBroker> broker;
    Handle executable(CreateFileW(o.exe.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, nullptr)); check(executable.h != INVALID_HANDLE_VALUE, "executable-lock"); check(hash(executable.h) == o.sha256, "executable-digest");
    Handle shell;
    if (!o.shell.empty()) { directories.push_back(directory_lock(o.shell)); shell.h = CreateFileW(o.shell.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, nullptr); check(shell.h != INVALID_HANDLE_VALUE, "shell-lock"); check(hash(shell.h) == o.shell_sha256, "shell-digest"); }
    Handle powershell;
    if (!o.powershell.empty()) { directories.push_back(directory_lock(o.powershell)); powershell.h = CreateFileW(o.powershell.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, nullptr); check(powershell.h != INVALID_HANDLE_VALUE, "powershell-lock"); check(hash(powershell.h) == o.powershell_sha256, "powershell-digest"); }
    Handle powershell_launcher;
    if (!o.powershell_launcher.empty()) {
        directories.push_back(directory_lock(o.powershell_launcher)); powershell_launcher.h = CreateFileW(o.powershell_launcher.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, nullptr);
        check(powershell_launcher.h != INVALID_HANDLE_VALUE, "powershell-launcher-lock"); check(hash(powershell_launcher.h) == o.powershell_launcher_sha256, "powershell-launcher-digest");
        auto actual = scope_final_path(powershell_launcher.h), real = scope_final_path(powershell.h);
        auto parent = actual.substr(0, actual.find_last_of(L'\\'));
        check(_wcsicmp(parent.substr(parent.find_last_of(L'\\') + 1).c_str(), L"powershell-launcher") == 0, "powershell-launcher-layout");
        auto expected = parent.substr(0, parent.find_last_of(L'\\')) + L"\\powershell\\pwsh.exe";
        check(_wcsicmp(real.c_str(), expected.c_str()) == 0, "powershell-engine-layout");
        for (auto& shim : lock_command_shims(actual, o.powershell_launcher_sha256)) directories.push_back(std::move(shim));
    }
    SECURITY_ATTRIBUTES sa{sizeof(sa), nullptr, TRUE}; Handle input_read, input_write, output_read, output_write, error_read, error_write;
    check(CreatePipe(&input_read.h, &input_write.h, &sa, 65536) && CreatePipe(&output_read.h, &output_write.h, &sa, 65536) && CreatePipe(&error_read.h, &error_write.h, &sa, 65536), "create-stdio");
    for (HANDLE owned : {input_write.h, output_read.h, error_read.h}) check(SetHandleInformation(owned, HANDLE_FLAG_INHERIT, 0) != 0, "stdio-inheritance");
    SIZE_T attribute_size = 0; InitializeProcThreadAttributeList(nullptr, 2, 0, &attribute_size); std::vector<uint8_t> attribute_storage(attribute_size);
    auto attributes = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(attribute_storage.data()); check(InitializeProcThreadAttributeList(attributes, 2, 0, &attribute_size) != 0, "attributes");
    check(UpdateProcThreadAttribute(attributes, 0, gent_psec::security_environment_attribute, &sandbox.value, sizeof(sandbox.value), nullptr, nullptr) != 0, "attach-psec");
    HANDLE inherited[] = {input_read.h, output_write.h, error_write.h}; check(UpdateProcThreadAttribute(attributes, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, inherited, sizeof(inherited), nullptr, nullptr) != 0, "stdio-handles");
    STARTUPINFOEXW startup{}; startup.StartupInfo.cb = sizeof(startup); startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES; startup.StartupInfo.hStdInput = input_read.h; startup.StartupInfo.hStdOutput = output_write.h; startup.StartupInfo.hStdError = error_write.h; startup.lpAttributeList = attributes;
    auto env = environment(o); std::wstring line = quote(o.exe); for (const auto& arg : o.args) line += L" " + quote(arg); check(line.size() < 32760, "command-length"); PROCESS_INFORMATION pi{};
    if(!o.state_root.empty() && !o.shell.empty()) {
        bool unassigned_settled=true;
        try {
            auto state=gent_environment_paths::ordinary(directory_lock(o.state_root).h);std::wstring bootstrap_line=quote(self)+L" bootstrap-temp "+quote(state+L"\\localappdata");PROCESS_INFORMATION bp{};
            check(CreateProcessW(self.c_str(),bootstrap_line.data(),nullptr,nullptr,TRUE,CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,env.data(),o.workspace.c_str(),&startup.StartupInfo,&bp)!=0,"bootstrap-create");
            Handle bootstrap(bp.hProcess), bootstrap_thread(bp.hThread);unassigned_settled=false;
            try { bind_suspended_to_job(bootstrap.h,bootstrap_thread.h,job.h);unassigned_settled=true; }
            catch (...) { unassigned_settled=TerminateProcess(bootstrap.h,126) && WaitForSingleObject(bootstrap.h,5000)==WAIT_OBJECT_0;throw; }
            check(ResumeThread(bootstrap_thread.h)!=static_cast<DWORD>(-1),"bootstrap-resume");
            check(WaitForSingleObject(bootstrap.h,10000)==WAIT_OBJECT_0,"bootstrap-timeout");
            DWORD bootstrap_code=1;check(GetExitCodeProcess(bootstrap.h,&bootstrap_code)&&bootstrap_code==0,"bootstrap-failed");
            check(wait_owned_job_empty(job.h,5000),"bootstrap-job-not-empty");
        } catch (...) {
            const bool provider_settled=TerminateJobObject(job.h,125) && wait_owned_job_empty(job.h,5000);
            const bool proxy_settled=!gateway || gateway->stop_and_wait(10000);
            check(unassigned_settled && provider_settled && proxy_settled,"bootstrap-settlement-unproven");
            std::cerr << "GENT-SANDBOX-ERROR bootstrap-failed:0\n";return 125;
        }
    }
    BOOL created = CreateProcessW(o.exe.c_str(), line.data(), nullptr, nullptr, TRUE, CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT, env.data(), o.workspace.c_str(), &startup.StartupInfo, &pi);
    DWORD creation_error = GetLastError(); DeleteProcThreadAttributeList(attributes); SetLastError(creation_error); check(created != 0, "psec-process-create");
    Handle child(pi.hProcess), thread(pi.hThread); input_read.close(); output_write.close(); error_write.close();
    try {
#ifdef GENT_FIXTURE_PAUSE_BEFORE_JOB
        FILETIME created_at{}, exited_at{}, kernel_time{}, user_time{}; check(GetProcessTimes(child.h, &created_at, &exited_at, &kernel_time, &user_time) != 0, "fixture-process-time");
        ULARGE_INTEGER created_value{}; created_value.LowPart = created_at.dwLowDateTime; created_value.HighPart = created_at.dwHighDateTime;
        std::string receipt = std::to_string(pi.dwProcessId) + ":" + std::to_string(created_value.QuadPart);
        auto receipt_path = o.state_root + L"\\suspended-process.fixture"; Handle receipt_file(CreateFileW(receipt_path.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW, FILE_FLAG_WRITE_THROUGH, nullptr)); check(receipt_file.h != INVALID_HANDLE_VALUE, "fixture-process-receipt"); write_all(receipt_file.h, receipt.data(), receipt.size()); receipt_file.close();
        Sleep(INFINITE);
#endif
        bind_suspended_to_job(child.h, thread.h, job.h);
        Handle token; check(OpenProcessToken(child.h, TOKEN_QUERY | TOKEN_DUPLICATE, &token.h) != 0, "token"); DWORD app = 0, size = 0; check(GetTokenInformation(token.h, TokenIsAppContainer, &app, sizeof(app), &size) && app == 1, "token-not-appcontainer");
        validate_readonly_authority(token.h, o.read, o.write);
        validate_provider_capabilities(token.h,o.gateway_port!=0);
        validate_low_integrity(token.h);
        if (o.gateway_port) broker = std::make_unique<CommandBroker>(command_broker_config(o, job.h, token.h, self));
        FILETIME creation{}, exit{}, kernel{}, user{}; check(GetProcessTimes(child.h, &creation, &exit, &kernel, &user) != 0, "creation-time"); ULARGE_INTEGER time{}; time.LowPart = creation.dwLowDateTime; time.HighPart = creation.dwHighDateTime;
        std::string proof = "GENT-SANDBOX/1 " + std::to_string(pi.dwProcessId) + ":" + std::to_string(time.QuadPart) + " " + utf8(o.profile) + " " + utf8(o.run) + (gateway ? gateway->evidence() : "") + "\n";
        check(!gateway || WaitForSingleObject(gateway->process(),0)==WAIT_TIMEOUT,"external-package-exited-before-resume");
        write_all(GetStdHandle(STD_OUTPUT_HANDLE), proof.data(), proof.size()); check(ResumeThread(thread.h) != static_cast<DWORD>(-1), "resume");
        std::thread out(relay, output_read.h, GetStdHandle(STD_OUTPUT_HANDLE)); std::thread err(relay, error_read.h, GetStdHandle(STD_ERROR_HANDLE));
        std::thread in([write_handle = std::move(input_write), owned_job = job.h]() mutable {
            relay(GetStdHandle(STD_INPUT_HANDLE), write_handle.h); write_handle.close();
            TerminateJobObject(owned_job, 126);
        });
        if (gateway) { HANDLE waits[]{child.h, gateway->process()}; if (WaitForMultipleObjects(2, waits, FALSE, INFINITE) == WAIT_OBJECT_0 + 1) TerminateJobObject(job.h, 126); }
        else WaitForSingleObject(child.h, INFINITE);
        DWORD code = 1; GetExitCodeProcess(child.h, &code);
        const bool commands_settled=!broker || broker->stop();
        const bool provider_settled=commands_settled && TerminateJobObject(job.h,code) && wait_owned_job_empty(job.h,5000);
        CancelSynchronousIo(in.native_handle());
        if(!provider_settled){CancelSynchronousIo(out.native_handle());CancelSynchronousIo(err.native_handle());}
        in.join();out.join();err.join();
        const bool proxy_settled=!gateway || gateway->stop_and_wait(10000);
        check(provider_settled,"provider-job-settlement-unproven");
        check(proxy_settled,"external-package-settlement-unproven");
        std::cerr << "GENT-SANDBOX-SETTLED " << utf8(o.run) << " exe=" << utf8(o.exe.substr(o.exe.find_last_of(L'\\') + 1)) << " " << job_accounting_text(job.h, "run_job") << (broker ? " " + broker->accounting() : std::string()) << "\n";
        return settled_scope_exit_code(code, provider_settled, proxy_settled);
    } catch (...) { TerminateProcess(child.h, 126); TerminateJobObject(job.h, 126); throw; }
}
int wmain(int argc, wchar_t** argv) {
    if (!SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)) return 126;
    try { check(argc >= 2, "mode"); std::wstring mode = argv[1];
        if (mode == L"bootstrap-temp") { check(argc == 3, "bootstrap-args"); return prepare_temp_inside(argv[2]); }
        if (mode == L"cleanup-peer") { check(argc == 6, "gateway-cleanup-options"); auto pid = std::stoull(argv[4]); check(pid && pid <= MAXDWORD, "gateway-cleanup-pid"); cleanup_gateway(argv[3], static_cast<DWORD>(pid), std::stoull(argv[5]), argv[2]); std::cout << "peer-cleaned\n"; return 0; }
        if (mode == L"cleanup-pending") { check(argc == 4, "gateway-pending-options"); cleanup_pending_gateway(argv[2], argv[3]); std::cout << "pending-peer-cleaned\n"; return 0; }
        if (mode == L"capabilities") { gent_psec::Api api; uint64_t flags = 0; hresult(api.query(&flags), "psec-support"); std::cout << "psec-v1-available support=" << flags << "; enforcement-unverified\n"; return 0; }
        if (mode == L"inspect-external-package") return inspect_external_package(argc, argv);
        if (mode == L"inspect-os-resources") return inspect_os_resources(argc, argv);
        if (mode == L"inspect-globalization-data") return inspect_globalization_data(argc, argv);
        if (mode == L"inspect-model-endpoint") return inspect_model_endpoint(argc, argv);
        if (mode == L"host-credential-probe") return run_credential_probe(argc,argv);
        if (mode == L"host-unlink-probe-handle") return gent_probe_unlink::run(argc,argv);
        if (mode == L"run") return run(parse(argc, argv)); fail("mode");
    } catch (const std::exception& error) { std::cerr << "GENT-SANDBOX-ERROR " << error.what() << "\n"; return 126; }
}
