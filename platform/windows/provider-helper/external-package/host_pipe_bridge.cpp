#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <windows.h>
#include <bcrypt.h>
#include <sddl.h>
#include <array>
#include <mutex>
#include <thread>
#include <vector>
#include "host_pipe_bridge.h"
#include "host_pipe_bridge_io.h"
#include "host_pipe_bridge_http.h"
#include "host_pipe_bridge_exchange.h"

using host_bridge::Handle;
using host_bridge::Io;
using host_bridge::Socket;
using host_bridge::require;

static std::wstring sid_text(PSID sid) {
    require(sid && IsValidSid(sid), "bridge-invalid-sid");
    wchar_t* text = nullptr;
    require(ConvertSidToStringSidW(sid, &text) != 0, "bridge-sid-text");
    std::wstring result(text); LocalFree(text); return result;
}
static std::vector<char> token_value(HANDLE token, TOKEN_INFORMATION_CLASS kind) {
    DWORD size = 0;
    GetTokenInformation(token, kind, nullptr, 0, &size);
    require(size && size <= 65536, "bridge-token-size");
    std::vector<char> value(size);
    require(GetTokenInformation(token, kind, value.data(), size, &size) != 0, "bridge-token-read");
    return value;
}
static bool exact_process(HANDLE process, DWORD pid, uint64_t creation, const std::wstring& sid) {
    if (GetProcessId(process) != pid || WaitForSingleObject(process, 0) != WAIT_TIMEOUT) return false;
    FILETIME created{}, exited{}, kernel{}, user{};
    if (!GetProcessTimes(process, &created, &exited, &kernel, &user)) return false;
    ULARGE_INTEGER time{}; time.LowPart = created.dwLowDateTime; time.HighPart = created.dwHighDateTime;
    if (time.QuadPart != creation) return false;
    HANDLE raw = nullptr;
    if (!OpenProcessToken(process, TOKEN_QUERY, &raw)) return false;
    Handle token(raw);
    DWORD app = 0, size = 0;
    if (!GetTokenInformation(token.value, TokenIsAppContainer, &app, sizeof(app), &size) || app != 1) return false;
    const auto value = token_value(token.value, TokenAppContainerSid);
    const auto information = reinterpret_cast<const TOKEN_APPCONTAINER_INFORMATION*>(value.data());
    return sid_text(information->TokenAppContainer) == sid;
}
struct HostPipeBridge::State {
    struct Slot {
        HANDLE pipe = INVALID_HANDLE_VALUE, client = INVALID_HANDLE_VALUE;
        std::thread worker;
        ~Slot() { if (pipe != INVALID_HANDLE_VALUE) CloseHandle(pipe); if (client != INVALID_HANDLE_VALUE) CloseHandle(client); }
    };
    uint16_t target;
    std::string broker_token;
    std::wstring name, peer_sid;
    Handle stopped, peer;
    DWORD pid = 0, session = 0;
    uint64_t creation = 0;
    std::array<Slot, 8> slots;
    std::mutex lifecycle;
    bool authorized = false, retired = false, winsock = false;

    State(uint16_t port, const std::wstring& nonce, const std::string& token)
        : target(port), broker_token(token), stopped(CreateEventW(nullptr, TRUE, FALSE, nullptr)) {
        require(port && nonce.size() == 32 && std::all_of(nonce.begin(), nonce.end(), [](wchar_t c) {
            return (c >= L'0' && c <= L'9') || (c >= L'a' && c <= L'f');
        }), "bridge-binding");
        require(token.empty() || host_bridge::hex(token, 64), "bridge-token-binding");
        require(stopped.value != nullptr, "bridge-stop-event");
        std::array<unsigned char, 16> random{};
        require(BCryptGenRandom(nullptr, random.data(), static_cast<ULONG>(random.size()), BCRYPT_USE_SYSTEM_PREFERRED_RNG) >= 0, "bridge-random");
        name = L"GentProvider-" + nonce + L"-";
        for (auto byte : random) { name += L"0123456789abcdef"[byte >> 4]; name += L"0123456789abcdef"[byte & 15]; }
        WSADATA data{};
        require(WSAStartup(MAKEWORD(2, 2), &data) == 0, "bridge-winsock"); winsock = true;
        try { prepare_pairs(); } catch (...) { WSACleanup(); winsock = false; throw; }
    }
    ~State() { stop(); if (winsock) WSACleanup(); }

    void prepare_pairs() {
        HANDLE raw = nullptr;
        require(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw) != 0, "bridge-owner-token");
        Handle token(raw); const auto owner = token_value(token.value, TokenUser);
        const auto owner_sid = sid_text(reinterpret_cast<const TOKEN_USER*>(owner.data())->User.Sid);
        const auto sddl = L"D:P(A;;GA;;;" + owner_sid + L")";
        PSECURITY_DESCRIPTOR descriptor = nullptr;
        require(ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(), SDDL_REVISION_1, &descriptor, nullptr) != 0, "bridge-pipe-security");
        SECURITY_ATTRIBUTES attributes{sizeof(SECURITY_ATTRIBUTES), descriptor, FALSE};
        try {
            for (size_t i = 0; i < slots.size(); ++i) {
                const auto endpoint = L"\\\\.\\pipe\\LOCAL\\" + name + L"-" + std::to_wstring(i);
                auto& slot = slots[i];
                slot.pipe = CreateNamedPipeW(endpoint.c_str(), PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 32768, 32768, 5000, &attributes);
                require(slot.pipe != INVALID_HANDLE_VALUE, "bridge-create-pair");
                Handle event(CreateEventW(nullptr, TRUE, FALSE, nullptr)); require(event.value != nullptr, "bridge-connect-event");
                OVERLAPPED operation{}; operation.hEvent = event.value;
                const auto connected = ConnectNamedPipe(slot.pipe, &operation);
                const auto error = connected ? ERROR_SUCCESS : GetLastError();
                require(connected || error == ERROR_IO_PENDING, "bridge-preconnect");
                slot.client = CreateFileW(endpoint.c_str(), FILE_READ_DATA | FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                    0, nullptr, OPEN_EXISTING, SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS, nullptr);
                const auto opened = slot.client != INVALID_HANDLE_VALUE;
                const auto signaled = connected || (opened && WaitForSingleObject(event.value, 5000) == WAIT_OBJECT_0);
                if (!signaled) CancelIoEx(slot.pipe, &operation);
                DWORD transferred = 0;
                const auto settled = connected || GetOverlappedResult(slot.pipe, &operation, &transferred, TRUE);
                require(opened && signaled && settled, "bridge-preconnect-settle");
                ULONG connector = 0, server = 0;
                require(GetNamedPipeClientProcessId(slot.pipe, &connector) && connector == GetCurrentProcessId()
                    && GetNamedPipeServerProcessId(slot.client, &server) && server == GetCurrentProcessId(), "bridge-host-pair-identity");
                require(SetHandleInformation(slot.client, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) != 0, "bridge-inheritance");
            }
        } catch (...) { LocalFree(descriptor); throw; }
        LocalFree(descriptor);
    }
    std::vector<HANDLE> inherited_handles() {
        std::lock_guard<std::mutex> guard(lifecycle);
        require(!authorized && !retired, "bridge-handoff-already-consumed");
        std::vector<HANDLE> result;
        for (const auto& slot : slots) { require(slot.client != INVALID_HANDLE_VALUE, "bridge-client-retired"); result.push_back(slot.client); }
        return result;
    }
    void capture_peer(PSID sid, DWORD process_id, uint64_t born, HANDLE job) {
        require(!authorized && !retired && process_id && born && job, "bridge-already-retired-or-authorized");
        authorized = true;
        peer_sid = sid_text(sid); pid = process_id; creation = born;
        peer.value = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_DUP_HANDLE | SYNCHRONIZE, FALSE, pid);
        require(peer.value && exact_process(peer.value, pid, creation, peer_sid), "bridge-peer-identity");
        DWORD own_session = 0; BOOL in_job = FALSE;
        require(ProcessIdToSessionId(pid, &session) && ProcessIdToSessionId(GetCurrentProcessId(), &own_session)
            && session == own_session && IsProcessInJob(peer.value, job, &in_job) && in_job, "bridge-peer-job-session");
    }
    void activate_workers() {
        try {
            for (auto& slot : slots) slot.worker = std::thread([this, &slot] { serve(slot); });
            for (auto& slot : slots) {
                require(CloseHandle(slot.client) != 0, "bridge-parent-copy-close"); slot.client = INVALID_HANDLE_VALUE;
            }
        } catch (...) { stop_unlocked(); throw; }
    }
    void authorize(PSID sid, DWORD process_id, uint64_t born, HANDLE job) {
        std::lock_guard<std::mutex> guard(lifecycle);
        capture_peer(sid, process_id, born, job);
        for (const auto& slot : slots) {
            HANDLE raw = nullptr;
            require(host_bridge::duplicate_remote(peer.value, slot.client, &raw), "bridge-inherited-handle-missing");
            Handle inherited(raw);
            require(CompareObjectHandles(slot.client, inherited.value) && GetFileType(inherited.value) == FILE_TYPE_PIPE, "bridge-inherited-object-mismatch");
        }
        activate_workers();
    }
    std::vector<HANDLE> mint(PSID sid, DWORD process_id, uint64_t born, HANDLE job) {
        std::lock_guard<std::mutex> guard(lifecycle);
        capture_peer(sid, process_id, born, job);
        std::vector<HANDLE> remote;
        try {
            remote.reserve(slots.size());
            for (const auto& slot : slots) {
                HANDLE transferred = nullptr;
                require(SetHandleInformation(slot.client, HANDLE_FLAG_INHERIT, 0) != 0, "bridge-host-private-handle");
                const auto minted = DuplicateHandle(GetCurrentProcess(), slot.client, peer.value, &transferred, 0, FALSE, DUPLICATE_SAME_ACCESS);
                const auto mint_error = minted ? ERROR_SUCCESS : GetLastError();
                host_bridge::require_native(minted, mint_error, "bridge-peer-mint");
                remote.push_back(transferred);
                HANDLE raw = nullptr;
                const auto copied = host_bridge::duplicate_remote(peer.value, transferred, &raw);
                const auto copy_error = copied ? ERROR_SUCCESS : GetLastError();
                host_bridge::require_native(copied, copy_error, "bridge-minted-readback");
                Handle verified(raw);
                SetLastError(ERROR_SUCCESS);
                const auto same = CompareObjectHandles(slot.client, verified.value);
                const auto compare_error = GetLastError();
                host_bridge::require_native(same, compare_error, "bridge-minted-object-mismatch");
                require(GetFileType(verified.value) == FILE_TYPE_PIPE, "bridge-minted-type-mismatch");
            }
            activate_workers();
            return remote;
        } catch (...) {
            bool settled = true;
            for (auto value : remote) if (!host_bridge::close_remote(peer.value, value)) settled = false;
            stop_unlocked();
            require(settled || WaitForSingleObject(peer.value, 0) == WAIT_OBJECT_0, "bridge-mint-rollback-unproven");
            throw;
        }
    }
    bool exchange(HANDLE pipe, const Io& io) {
        return host_bridge::exchange(pipe, io, target, broker_token);
    }
    void serve(Slot& slot) noexcept {
        const Io io{stopped.value, peer.value};
        try { while (io.active() && exchange(slot.pipe, io)) {} } catch (...) {}
        DisconnectNamedPipe(slot.pipe); CloseHandle(slot.pipe); slot.pipe = INVALID_HANDLE_VALUE;
    }
    void stop_unlocked() {
        retired = true; SetEvent(stopped.value);
        for (auto& slot : slots) if (slot.worker.joinable()) slot.worker.join();
        for (auto& slot : slots) if (slot.client != INVALID_HANDLE_VALUE) { CloseHandle(slot.client); slot.client = INVALID_HANDLE_VALUE; }
        for (auto& slot : slots) if (slot.pipe != INVALID_HANDLE_VALUE) {
            CloseHandle(slot.pipe); slot.pipe = INVALID_HANDLE_VALUE;
        }
        if (!broker_token.empty()) SecureZeroMemory(broker_token.data(), broker_token.size());
        broker_token.clear();
    }
    void stop() { std::lock_guard<std::mutex> guard(lifecycle); stop_unlocked(); }
};

HostPipeBridge::HostPipeBridge(uint16_t port, const std::wstring& nonce, const std::string& token)
    : state(std::make_unique<State>(port, nonce, token)) {}
HostPipeBridge::~HostPipeBridge() = default;
std::vector<HANDLE> HostPipeBridge::inherited_handles() const { return state->inherited_handles(); }
std::vector<HANDLE> HostPipeBridge::mint_peer_handles(PSID sid, DWORD pid, uint64_t birth, HANDLE job) { return state->mint(sid, pid, birth, job); }
void HostPipeBridge::authorize_peer(PSID sid, DWORD pid, uint64_t creation, HANDLE job) { state->authorize(sid, pid, creation, job); }
void HostPipeBridge::stop() { state->stop(); }

HANDLE HostPipeBridge::failure_event() const { return state->stopped.value; }
