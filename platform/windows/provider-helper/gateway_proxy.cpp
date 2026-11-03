#include <algorithm>
#include "gateway_proxy.h"
#include <bcrypt.h>
#include <sddl.h>
#include <userenv.h>
#include <shlobj.h>
#include <array>
#include <stdexcept>
#include <vector>

static void check(bool ok, const char* what) { if (!ok) throw std::runtime_error(std::string(what) + ":" + std::to_string(GetLastError())); }
static void validate_state_path(const std::wstring& root) {
    size_t first = root.rfind(L"\\\\?\\", 0) == 0 ? 7 : 3;
    check(root.size() > first && ((first == 3 && root[1] == L':') || (first == 7 && root[5] == L':')), "gateway-state-path");
    for (size_t i = first, start = first; i <= root.size(); ++i) {
        if (i != root.size() && root[i] != L'\\' && root[i] != L'/') continue;
        auto part = root.substr(start, i - start); check(!part.empty() && part != L"." && part != L"..", "gateway-state-components"); start = i + 1;
        DWORD attr = GetFileAttributesW(root.substr(0, i).c_str());
        if (attr == INVALID_FILE_ATTRIBUTES) { check(GetLastError() == ERROR_FILE_NOT_FOUND || GetLastError() == ERROR_PATH_NOT_FOUND, "gateway-state-query"); break; }
        check((attr & FILE_ATTRIBUTE_DIRECTORY) && !(attr & FILE_ATTRIBUTE_REPARSE_POINT), "gateway-state-reparse");
    }
}
struct GatewayProxy::State {
    std::wstring name, directory, image;
    HANDLE process = nullptr;
    HANDLE job = nullptr;
    HANDLE directory_lock = INVALID_HANDLE_VALUE, image_lock = INVALID_HANDLE_VALUE, receipt_lock = INVALID_HANDLE_VALUE;
    uint16_t endpoint = 0;
    uint64_t creation = 0;
    DWORD pid = 0;
    bool profile_created = false;
    ~State() {
        if (process) { TerminateProcess(process, 0); WaitForSingleObject(process, 5000); CloseHandle(process); }
        if (job) CloseHandle(job);
        if (receipt_lock != INVALID_HANDLE_VALUE) CloseHandle(receipt_lock);
        if (image_lock != INVALID_HANDLE_VALUE) CloseHandle(image_lock);
        if (directory_lock != INVALID_HANDLE_VALUE) CloseHandle(directory_lock);
        if (profile_created) DeleteAppContainerProfile(name.c_str());
        if (!directory.empty()) { DeleteFileW((directory + L"\\.ownership").c_str()); DeleteFileW((directory + L"\\.ownership.new").c_str()); }
        if (!image.empty()) DeleteFileW(image.c_str());
        if (!directory.empty()) RemoveDirectoryW(directory.c_str());
    }
};
static std::wstring quote(const std::wstring& value) {
    std::wstring result = L"\""; size_t slash = 0;
    for (auto c : value) { if (c == L'\\') { ++slash; continue; } result.append(c == L'"' ? slash * 2 + 1 : slash, L'\\'); slash = 0; result += c; }
    result.append(slash * 2, L'\\'); return result + L'"';
}
GatewayProxy::GatewayProxy(uint16_t target_port, const std::wstring& executable, const std::wstring& state_root, const std::wstring& nonce, const std::wstring& broker_token) : state(std::make_unique<State>()) {
    check(broker_token.empty() || (broker_token.size() == 64 && std::all_of(broker_token.begin(), broker_token.end(), [](wchar_t c) { return (c >= L'0' && c <= L'9') || (c >= L'a' && c <= L'f'); })), "gateway-broker-token");
    validate_state_path(state_root);
    state->job = CreateJobObjectW(nullptr, nullptr); check(state->job != nullptr, "gateway-job-create");
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{}; limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_JOB_MEMORY;
    limits.BasicLimitInformation.ActiveProcessLimit = 1; limits.JobMemoryLimit = 64 * 1024 * 1024;
    check(SetInformationJobObject(state->job, JobObjectExtendedLimitInformation, &limits, sizeof(limits)) != 0, "gateway-job-limits");
    check(nonce.size() == 32, "gateway-nonce"); for (auto c : nonce) check((c >= L'0' && c <= L'9') || (c >= L'a' && c <= L'f'), "gateway-nonce"); state->name = L"GentGateway-" + nonce;
    PSID sid = nullptr; check(SUCCEEDED(CreateAppContainerProfile(state->name.c_str(), L"Gent gateway", L"Gent gateway", nullptr, 0, &sid)), "gateway-profile"); state->profile_created = true;
    LPWSTR sid_text = nullptr; check(ConvertSidToStringSidW(sid, &sid_text) != 0, "gateway-sid"); std::wstring package_sid = sid_text; LocalFree(sid_text);
    state->directory = state_root + L"\\" + state->name;
    PSECURITY_DESCRIPTOR security = nullptr;
    auto directory_sddl = L"D:P(A;OICI;FA;;;OW)(A;OICI;FRFX;;;" + package_sid + L")";
    check(ConvertStringSecurityDescriptorToSecurityDescriptorW(directory_sddl.c_str(), SDDL_REVISION_1, &security, nullptr) != 0, "gateway-directory-security");
    SECURITY_ATTRIBUTES attributes{sizeof(attributes), security, FALSE}; check(CreateDirectoryW(state->directory.c_str(), &attributes) != 0, "gateway-directory");
    state->image = state->directory + L"\\gateway.exe"; check(CopyFileW(executable.c_str(), state->image.c_str(), TRUE) != 0, "gateway-image");
    check(SetFileSecurityW(state->image.c_str(), DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, security) != 0, "gateway-image-security"); LocalFree(security);
    state->directory_lock = CreateFileW(state->directory.c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, nullptr); check(state->directory_lock != INVALID_HANDLE_VALUE, "gateway-directory-lock");
    state->image_lock = CreateFileW(state->image.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, 0, nullptr); check(state->image_lock != INVALID_HANDLE_VALUE, "gateway-image-lock");
    DWORD session = 0; check(ProcessIdToSessionId(GetCurrentProcessId(), &session) != 0, "gateway-session");
    auto server_name = L"\\\\.\\pipe\\Sessions\\" + std::to_wstring(session) + L"\\AppContainerNamedObjects\\" + package_sid + L"\\" + state->name;
    auto client_name = L"\\\\.\\pipe\\LOCAL\\" + state->name;
    auto pipe_sddl = L"D:P(A;;GA;;;OW)(A;;GA;;;" + package_sid + L")S:(ML;;NW;;;LW)";
    check(ConvertStringSecurityDescriptorToSecurityDescriptorW(pipe_sddl.c_str(), SDDL_REVISION_1, &security, nullptr) != 0, "gateway-pipe-security");
    attributes.lpSecurityDescriptor = security;
    HANDLE pipe = CreateNamedPipeW(server_name.c_str(), PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 64, 64, 0, &attributes); LocalFree(security); check(pipe != INVALID_HANDLE_VALUE, "gateway-status-pipe");
    PSID* capability_sids = nullptr; PSID* group_sids = nullptr; DWORD capability_count = 0, group_count = 0;
    check(DeriveCapabilitySidsFromName(L"networkLoopback", &group_sids, &group_count, &capability_sids, &capability_count) && capability_count == 1, "gateway-capability");
    PSID private_sid = nullptr; check(ConvertStringSidToSidW(L"S-1-15-3-3", &private_sid) != 0, "gateway-private-capability");
    SID_AND_ATTRIBUTES capability[]{{capability_sids[0], SE_GROUP_ENABLED}, {private_sid, SE_GROUP_ENABLED}}; SECURITY_CAPABILITIES capabilities{sid, capability, 2, 0};
    SIZE_T bytes = 0; InitializeProcThreadAttributeList(nullptr, 2, 0, &bytes); std::vector<uint8_t> storage(bytes); auto list = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(storage.data());
    check(InitializeProcThreadAttributeList(list, 2, 0, &bytes) != 0, "gateway-attributes");
    check(UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities, sizeof(capabilities), nullptr, nullptr) != 0, "gateway-container-attribute");
    check(UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_JOB_LIST, &state->job, sizeof(state->job), nullptr, nullptr) != 0, "gateway-atomic-job");
    STARTUPINFOEXW startup{}; startup.StartupInfo.cb = sizeof(startup); startup.lpAttributeList = list;
    auto command = quote(state->image) + L" gateway-proxy " + std::to_wstring(target_port) + L" " + quote(client_name); PROCESS_INFORMATION pi{};
    if (!broker_token.empty()) command += L" " + quote(broker_token);
    PWSTR local_app_data = nullptr; check(SUCCEEDED(SHGetKnownFolderPath(FOLDERID_LocalAppData, 0, nullptr, &local_app_data)), "gateway-local-appdata");
    std::wstring environment = L"LOCALAPPDATA=" + std::wstring(local_app_data); CoTaskMemFree(local_app_data); environment.push_back(0);
    std::wstring windows(32768, L'\0'); UINT windows_length = GetWindowsDirectoryW(windows.data(), static_cast<UINT>(windows.size())); check(windows_length && windows_length < windows.size(), "gateway-windows-directory"); windows.resize(windows_length);
    environment += L"SystemRoot=" + windows; environment.push_back(0); environment += L"windir=" + windows; environment.push_back(0); environment.push_back(0);
    BOOL created = CreateProcessW(state->image.c_str(), command.data(), nullptr, nullptr, FALSE, CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT, environment.data(), state->directory.c_str(), &startup.StartupInfo, &pi);
    DeleteProcThreadAttributeList(list); for (DWORD i = 0; i < capability_count; ++i) LocalFree(capability_sids[i]); LocalFree(capability_sids); for (DWORD i = 0; i < group_count; ++i) LocalFree(group_sids[i]); LocalFree(group_sids); LocalFree(private_sid); FreeSid(sid);
    check(created != 0, "gateway-spawn"); state->process = pi.hProcess; state->pid = pi.dwProcessId;
    BOOL in_job = FALSE; check(IsProcessInJob(pi.hProcess, state->job, &in_job) && in_job, "gateway-job-membership");
    FILETIME creation{}, exit{}, kernel{}, user{}; check(GetProcessTimes(pi.hProcess, &creation, &exit, &kernel, &user) != 0, "gateway-creation");
    ULARGE_INTEGER time{}; time.LowPart = creation.dwLowDateTime; time.HighPart = creation.dwHighDateTime; state->creation = time.QuadPart;
    std::string ascii_name; for (auto c : state->name) ascii_name += static_cast<char>(c);
    std::string receipt = "GENT-GATEWAY-OWNERSHIP/1\n" + ascii_name + "\n" + std::to_string(state->pid) + "\n" + std::to_string(state->creation) + "\n";
    auto receipt_path = state->directory + L"\\.ownership", pending_receipt = receipt_path + L".new";
    HANDLE receipt_file = CreateFileW(pending_receipt.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW, FILE_FLAG_WRITE_THROUGH, nullptr); check(receipt_file != INVALID_HANDLE_VALUE, "gateway-ownership-create");
    DWORD receipt_written = 0; BOOL receipt_ok = WriteFile(receipt_file, receipt.data(), static_cast<DWORD>(receipt.size()), &receipt_written, nullptr); CloseHandle(receipt_file); check(receipt_ok && receipt_written == receipt.size(), "gateway-ownership-write");
    check(MoveFileExW(pending_receipt.c_str(), receipt_path.c_str(), MOVEFILE_WRITE_THROUGH) != 0, "gateway-ownership-commit");
    state->receipt_lock = CreateFileW(receipt_path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, 0, nullptr); check(state->receipt_lock != INVALID_HANDLE_VALUE, "gateway-ownership-lock");
    check(ResumeThread(pi.hThread) != static_cast<DWORD>(-1), "gateway-resume"); CloseHandle(pi.hThread);
    HANDLE event = CreateEventW(nullptr, TRUE, FALSE, nullptr); OVERLAPPED operation{}; operation.hEvent = event; DWORD n = 0;
    BOOL connected = ConnectNamedPipe(pipe, &operation); DWORD error = connected ? ERROR_SUCCESS : GetLastError();
    if (error == ERROR_IO_PENDING) { HANDLE waits[]{event, state->process}; DWORD result = WaitForMultipleObjects(2, waits, FALSE, 10000);
        if (result != WAIT_OBJECT_0) { CancelIoEx(pipe, &operation); WaitForSingleObject(event, INFINITE); CloseHandle(event); CloseHandle(pipe); throw std::runtime_error("gateway-rendezvous:" + std::to_string(result)); }
        check(GetOverlappedResult(pipe, &operation, &n, FALSE) != 0, "gateway-connect"); }
    else check(error == ERROR_SUCCESS || error == ERROR_PIPE_CONNECTED, "gateway-connect");
    ULONG client = 0; check(GetNamedPipeClientProcessId(pipe, &client) && client == state->pid, "gateway-peer");
    std::array<char, 16> reply{}; ResetEvent(event); operation = {}; operation.hEvent = event;
    BOOL read = ReadFile(pipe, reply.data(), static_cast<DWORD>(reply.size() - 1), &n, &operation);
    if (!read && GetLastError() == ERROR_IO_PENDING) { if (WaitForSingleObject(event, 10000) != WAIT_OBJECT_0) { CancelIoEx(pipe, &operation); WaitForSingleObject(event, INFINITE); CloseHandle(event); CloseHandle(pipe); throw std::runtime_error("gateway-ready-timeout"); } read = GetOverlappedResult(pipe, &operation, &n, FALSE); }
    CloseHandle(event); CloseHandle(pipe); check(read && n > 1 && reply[n - 1] == '\n', "gateway-ready"); reply[n - 1] = 0;
    unsigned long port = std::stoul(reply.data()); check(port && port <= 65535, "gateway-endpoint"); state->endpoint = static_cast<uint16_t>(port);
}
GatewayProxy::~GatewayProxy() = default;
const std::wstring& GatewayProxy::moniker() const { return state->name; }
const std::wstring& GatewayProxy::runtime() const { return state->directory; }
uint16_t GatewayProxy::port() const { return state->endpoint; }
HANDLE GatewayProxy::process() const { return state->process; }
std::string GatewayProxy::evidence() const {
    std::string name; for (auto c : state->name) name += static_cast<char>(c);
    return " peer=" + name + "," + std::to_string(state->endpoint) + "," + std::to_string(state->pid) + "," + std::to_string(state->creation);
}
void cleanup_gateway(const std::wstring& moniker, DWORD pid, uint64_t creation, const std::wstring& state_root) {
    validate_state_path(state_root);
    check(moniker.size() == 44 && moniker.rfind(L"GentGateway-", 0) == 0 && pid && creation, "gateway-cleanup-binding");
    for (size_t i = 12; i < moniker.size(); ++i) check((moniker[i] >= L'0' && moniker[i] <= L'9') || (moniker[i] >= L'a' && moniker[i] <= L'f'), "gateway-cleanup-name");
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, FALSE, pid);
    if (process) {
        FILETIME created{}, exited{}, kernel{}, user{}; BOOL queried = GetProcessTimes(process, &created, &exited, &kernel, &user); DWORD error = GetLastError();
        ULARGE_INTEGER time{}; time.LowPart = created.dwLowDateTime; time.HighPart = created.dwHighDateTime;
        bool exited_process = WaitForSingleObject(process, 0) == WAIT_OBJECT_0;
        if (queried && time.QuadPart == creation && !exited_process) exited_process = WaitForSingleObject(process, 750) == WAIT_OBJECT_0;
        CloseHandle(process); SetLastError(error); check(queried != 0, "gateway-cleanup-process"); check(time.QuadPart != creation || exited_process, "gateway-cleanup-still-live");
    } else check(GetLastError() == ERROR_INVALID_PARAMETER, "gateway-cleanup-unknown-process");
    check(state_root.size() > 3 && (state_root[1] == L':' || state_root.rfind(L"\\\\?\\", 0) == 0), "gateway-cleanup-state");
    auto directory = state_root + L"\\" + moniker; DWORD attributes = GetFileAttributesW(directory.c_str());
    if (attributes != INVALID_FILE_ATTRIBUTES) {
        check((attributes & FILE_ATTRIBUTE_DIRECTORY) && !(attributes & FILE_ATTRIBUTE_REPARSE_POINT), "gateway-cleanup-reparse");
        auto image = directory + L"\\gateway.exe"; attributes = GetFileAttributesW(image.c_str());
        if (attributes != INVALID_FILE_ATTRIBUTES) { check(!(attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)), "gateway-cleanup-image"); check(DeleteFileW(image.c_str()) != 0, "gateway-cleanup-image-delete"); }
        for (const auto& name : {L".ownership", L".ownership.new"}) { auto receipt = directory + L"\\" + name; attributes = GetFileAttributesW(receipt.c_str());
            if (attributes != INVALID_FILE_ATTRIBUTES) { check(!(attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)), "gateway-cleanup-ownership-reparse"); check(DeleteFileW(receipt.c_str()) != 0, "gateway-cleanup-ownership-delete"); } }
        check(RemoveDirectoryW(directory.c_str()) != 0, "gateway-cleanup-directory-delete");
    } else check(GetLastError() == ERROR_FILE_NOT_FOUND || GetLastError() == ERROR_PATH_NOT_FOUND, "gateway-cleanup-directory");
    HRESULT result = DeleteAppContainerProfile(moniker.c_str()); check(SUCCEEDED(result) || result == HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND) || result == HRESULT_FROM_WIN32(ERROR_NOT_FOUND), "gateway-cleanup-profile");
}
void cleanup_pending_gateway(const std::wstring& state_root, const std::wstring& moniker) {
    validate_state_path(state_root);
    check(moniker.size() == 44 && moniker.rfind(L"GentGateway-", 0) == 0, "gateway-pending-name");
    for (size_t i = 12; i < moniker.size(); ++i) check((moniker[i] >= L'0' && moniker[i] <= L'9') || (moniker[i] >= L'a' && moniker[i] <= L'f'), "gateway-pending-name");
    auto directory = state_root + L"\\" + moniker, path = directory + L"\\.ownership";
    DWORD attrs = GetFileAttributesW(directory.c_str()); check(attrs != INVALID_FILE_ATTRIBUTES && !(attrs & FILE_ATTRIBUTE_REPARSE_POINT), "gateway-pending-directory");
    attrs = GetFileAttributesW(path.c_str()); check(attrs != INVALID_FILE_ATTRIBUTES && !(attrs & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)), "gateway-pending-ownership");
    HANDLE file = CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr); check(file != INVALID_HANDLE_VALUE, "gateway-pending-read");
    LARGE_INTEGER size{}; BOOL sized = GetFileSizeEx(file, &size); std::array<char, 256> bytes{}; DWORD read = 0;
    BOOL read_ok = sized && size.QuadPart > 0 && size.QuadPart < static_cast<LONGLONG>(bytes.size()) && ReadFile(file, bytes.data(), static_cast<DWORD>(size.QuadPart), &read, nullptr); CloseHandle(file); check(read_ok && read == static_cast<DWORD>(size.QuadPart), "gateway-pending-size");
    std::string receipt(bytes.data(), read), expected_name; for (auto c : moniker) expected_name += static_cast<char>(c);
    auto first = receipt.find('\n'), second = receipt.find('\n', first + 1), third = receipt.find('\n', second + 1), fourth = receipt.find('\n', third + 1);
    check(first != std::string::npos && second != std::string::npos && third != std::string::npos && fourth == receipt.size() - 1 && receipt.substr(0, first) == "GENT-GATEWAY-OWNERSHIP/1" && receipt.substr(first + 1, second - first - 1) == expected_name, "gateway-pending-shape");
    size_t consumed = 0; auto pid_text = receipt.substr(second + 1, third - second - 1); auto pid = std::stoull(pid_text, &consumed); check(pid && pid <= MAXDWORD && consumed == pid_text.size(), "gateway-pending-pid");
    auto creation_text = receipt.substr(third + 1, fourth - third - 1); auto creation = std::stoull(creation_text, &consumed); check(creation && consumed == creation_text.size(), "gateway-pending-creation");
    cleanup_gateway(moniker, static_cast<DWORD>(pid), creation, state_root);
}
