#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <windows.h>
#include <bcrypt.h>
#include <shobjidl.h>
#include <array>
#include <thread>
#include <mutex>
#include <stdexcept>
#include "external_package_proxy.h"
#include "package_authority.h"
#include "package_owner.h"
#include "package_control.h"
#include "external_channel_client.h"
#include "host_pipe_bridge.h"
namespace external_bridge {
namespace {
void require(bool ok, const char* stage) { if (!ok) throw std::runtime_error(stage); }
std::string ascii(const std::wstring& value) {
    std::string result;
    for (const auto c : value) { require(c > 32 && c < 127 && c != L',', "package-ascii-field"); result += static_cast<char>(c); }
    return result;
}
std::wstring quote(const std::wstring& value) {
    std::wstring result = L"\""; size_t slashes = 0;
    for (const auto c : value) {
        if (c == L'\\') { ++slashes; continue; }
        result.append(c == L'\"' ? slashes * 2 + 1 : slashes, L'\\'); slashes = 0; result += c;
    }
    result.append(slashes * 2, L'\\'); return result + L"\"";
}
std::string random_bearer() {
    std::array<unsigned char, 32> bytes{};
    require(BCryptGenRandom(nullptr, bytes.data(), static_cast<ULONG>(bytes.size()),
        BCRYPT_USE_SYSTEM_PREFERRED_RNG) >= 0, "package-private-bearer-random");
    std::string result;
    for (const auto byte : bytes) { result += "0123456789abcdef"[byte >> 4]; result += "0123456789abcdef"[byte & 15]; }
    SecureZeroMemory(bytes.data(), bytes.size()); return result;
}
}
struct ExternalPackageProxy::State {
    PackageAuthority authority;
    PackageOwner owner;
    std::wstring nonce;
    std::string bearer;
    std::unique_ptr<HostPipeBridge> bridge;
    HANDLE peer = nullptr, stopping = nullptr, admission_record = INVALID_HANDLE_VALUE;
    DWORD pid = 0; uint64_t birth = 0; uint16_t listener = 0;
    bool attempted = false, admitted = false, recorded_settled = false;
    std::thread watcher;
    std::mutex lifecycle;
    State(const std::wstring& root, const std::wstring& run)
        : owner(authority.family, root, run), nonce(run), bearer(random_bearer()) {
        stopping = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        require(stopping != nullptr, "package-stopping-event");
    }
    ~State() {
        if (watcher.joinable()) watcher.join();
        if (peer) CloseHandle(peer);
        if (stopping) CloseHandle(stopping);
        if (admission_record != INVALID_HANDLE_VALUE) CloseHandle(admission_record);
        if (!bearer.empty()) SecureZeroMemory(bearer.data(), bearer.size());
    }
    void start(uint16_t upstream, const std::string& broker_token) {
        require(upstream && !broker_token.empty() && broker_token != bearer, "package-distinct-auth-binding");
        bridge = std::make_unique<HostPipeBridge>(upstream, nonce, broker_token);
        owner.record("Artifact " + ascii(authority.full_name) + " " + authority.image_digest
            + " " + authority.manifest_digest + "\n");
        require(SUCCEEDED(CoInitializeEx(nullptr, COINIT_MULTITHREADED)), "package-activation-com");
        IApplicationActivationManager* manager = nullptr;
        HRESULT activation = E_FAIL; DWORD activated_pid = 0; FILETIME before{};
        const auto args = quote(owner.observation) + L" " + quote(owner.descriptor) + L" " + nonce
            + L" " + std::to_wstring(GetCurrentProcessId()) + L" "
            + std::to_wstring(process_birth(GetCurrentProcess())) + L" " + std::to_wstring(owner.session);
        if (SUCCEEDED(CoCreateInstance(CLSID_ApplicationActivationManager, nullptr, CLSCTX_LOCAL_SERVER,
                IID_PPV_ARGS(&manager)))) {
            GetSystemTimeAsFileTime(&before); attempted = true;
            activation = manager->ActivateApplication(authority.aumid.c_str(), args.c_str(),
                AO_NOERRORUI | AO_NOSPLASHSCREEN, &activated_pid);
            manager->Release();
        }
        CoUninitialize();
        require(SUCCEEDED(activation) && activated_pid, "package-activation-unknown-or-failed");
        peer = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_DUP_HANDLE | SYNCHRONIZE
            | PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, activated_pid);
        require(peer != nullptr, "package-owned-process-open");
        pid = activated_pid; birth = process_birth(peer);
        const uint64_t before_time = static_cast<uint64_t>(before.dwLowDateTime)
            | (static_cast<uint64_t>(before.dwHighDateTime) << 32);
        require(birth && birth >= before_time, "package-activation-returned-preexisting-owner");
        PackageGrant grant{pid, birth, owner.session, authority.family, authority.full_name,
            owner.sid_text, authority.image};
        HANDLE verified = capture_package_peer(grant);
        const bool exact = CompareObjectHandles(peer, verified) != 0; CloseHandle(verified);
        require(exact, "package-activation-object-mismatch");
        owner.record("Observed " + std::to_string(pid) + " " + std::to_string(birth) + "\n");
        // Broker activation may otherwise return a different live instance.
        // Require this immutable pinned executable to acknowledge our exact
        // owner/run in the fresh generation before mutating its Job/handles.
        const auto admission_deadline = GetTickCount64() + 10000;
        DWORD admission_error = ERROR_FILE_NOT_FOUND;
        while (GetTickCount64() < admission_deadline && WaitForSingleObject(peer, 0) == WAIT_TIMEOUT) {
            admission_record = CreateFileW((owner.observation + L".admission").c_str(), GENERIC_READ,
                FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
            admission_error = admission_record == INVALID_HANDLE_VALUE ? GetLastError() : ERROR_SUCCESS;
            if (admission_record != INVALID_HANDLE_VALUE) break;
            Sleep(10);
        }
        if (admission_record == INVALID_HANDLE_VALUE) {
            DWORD code = 0; const bool observed_exit = GetExitCodeProcess(peer, &code) != 0;
            throw std::runtime_error("package-activation-no-run-admission:" + std::to_string(admission_error)
                + ":peer-exit:" + (observed_exit ? std::to_string(code) : "unknown"));
        }
        FILE_ATTRIBUTE_TAG_INFO attributes{}; LARGE_INTEGER size{};
        require(GetFileInformationByHandleEx(admission_record, FileAttributeTagInfo, &attributes, sizeof(attributes))
            && !(attributes.FileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT))
            && GetFileSizeEx(admission_record, &size) && size.QuadPart > 0 && size.QuadPart <= 512,
            "package-activation-admission-file");
        std::string actual(static_cast<size_t>(size.QuadPart), '\0'); DWORD read = 0;
        require(ReadFile(admission_record, actual.data(), static_cast<DWORD>(actual.size()), &read, nullptr)
            && read == actual.size(), "package-activation-admission-read");
        const auto expected = "GENT-PACKAGE-ADMISSION/1 " + std::to_string(pid) + " "
            + std::to_string(birth) + " " + std::to_string(GetCurrentProcessId()) + " "
            + std::to_string(process_birth(GetCurrentProcess())) + " " + std::to_string(owner.session)
            + " " + ascii(nonce) + "\n";
        require(actual == expected, "package-activation-foreign-run");
        // Only a fresh natively authenticated activation may enter our Job.
        admitted = true;
        require(AssignProcessToJobObject(owner.job, peer), "package-owned-job-assign");
        BOOL contained = FALSE;
        require(IsProcessInJob(peer, owner.job, &contained) && contained, "package-owned-job-exact");
        owner.record("Admitted " + std::to_string(pid) + " " + std::to_string(birth) + "\n");
        const auto channels = bridge->mint_peer_handles(owner.sid, pid, birth, owner.job);
        listener = start_package_control(peer, grant, owner.descriptor, ascii(nonce), channels, bearer);
        require(WaitForSingleObject(peer, 0) == WAIT_TIMEOUT
            && WaitForSingleObject(bridge->failure_event(), 0) == WAIT_TIMEOUT, "package-startup-retired");
        owner.record("Ready " + std::to_string(listener) + "\n");
        watcher = std::thread([this] {
            HANDLE events[]{stopping, peer, bridge->failure_event()};
            if (WaitForMultipleObjects(3, events, FALSE, INFINITE) == WAIT_OBJECT_0 + 2)
                TerminateJobObject(owner.job, ERROR_BROKEN_PIPE);
        });
    }
    bool settle(DWORD timeout) {
        std::lock_guard<std::mutex> lock(lifecycle);
        SetEvent(stopping);
        if (bridge) bridge->stop();
        if (watcher.joinable()) watcher.join();
        // Never terminate an existing/different process merely returned by
        // activation. Unknown activation is retained for explicit recovery.
        if (attempted && !admitted) return false;
        if (admitted && peer && WaitForSingleObject(peer, 0) == WAIT_TIMEOUT) {
            if (!TerminateJobObject(owner.job, ERROR_CANCELLED)) return false;
        }
        const auto deadline = GetTickCount64() + timeout;
        if (peer && WaitForSingleObject(peer, timeout) != WAIT_OBJECT_0) return false;
        while (!owner.job_empty() && GetTickCount64() < deadline) Sleep(10);
        if (!owner.job_empty()) return false;
        if (!recorded_settled) {
            try { owner.record("Settled\n"); recorded_settled = true; }
            catch (...) { return false; }
        }
        return true;
    }
};
// Unknown settlement is an actual retained owner, not a local RAII comment.
// No replay/retry is performed. The durable receipt and all pins/Job handles
// remain held for this owner-process lifetime. No in-process recovery/replay API
// is implied; the durable unknown receipt remains for external reconciliation.
static void retain_unsettled(void* state) {
    static auto* lock = new std::mutex;
    std::lock_guard<std::mutex> guard(*lock);
    static auto* retained = new std::vector<void*>;
    retained->push_back(state);
}
ExternalPackageProxy::ExternalPackageProxy(uint16_t port, const std::wstring& root,
    const std::wstring& nonce, const std::string& broker_token)
    : state(std::make_unique<State>(root, nonce)) {
    try { state->start(port, broker_token); }
    catch (...) {
        try { if (!state->settle(10000)) retain_unsettled(state.release()); }
        catch (...) { retain_unsettled(state.release()); }
        throw;
    }
}
ExternalPackageProxy::~ExternalPackageProxy() {
    try { if (state && !state->settle(10000)) retain_unsettled(state.release()); }
    catch (...) { retain_unsettled(state.release()); }
}
const std::wstring& ExternalPackageProxy::family() const { return state->authority.family; }
const std::wstring& ExternalPackageProxy::full_name() const { return state->authority.full_name; }
const std::string& ExternalPackageProxy::run_bearer() const { return state->bearer; }
const std::wstring& ExternalPackageProxy::control_root() const { return state->owner.control_root; }
uint16_t ExternalPackageProxy::port() const { return state->listener; }
DWORD ExternalPackageProxy::process_id() const { return state->pid; }
uint64_t ExternalPackageProxy::creation_time() const { return state->birth; }
HANDLE ExternalPackageProxy::process() const { return state->peer; }
bool ExternalPackageProxy::stop_and_wait(DWORD timeout) { return state->settle(timeout); }
std::string ExternalPackageProxy::evidence() const {
    return " package=" + ascii(state->authority.family) + "," + ascii(state->authority.full_name)
        + "," + ascii(state->nonce) + "," + std::to_string(state->listener) + ","
        + std::to_string(state->pid) + "," + std::to_string(state->birth);
}
}
