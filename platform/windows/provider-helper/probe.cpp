#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <string>
#include <array>
#include <cstdio>
#include <vector>
#include "probe_network.h"
#include "probe_resources.h"

static bool read(const wchar_t* path) {
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, 0, nullptr);
    if (h == INVALID_HANDLE_VALUE) return false;
    char value; DWORD n = 0; BOOL ok = ReadFile(h, &value, 1, &n, nullptr); CloseHandle(h); return ok && n;
}
static bool write(const wchar_t* path) {
    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, 0, nullptr);
    if (h == INVALID_HANDLE_VALUE) return false;
    DWORD n = 0; BOOL ok = WriteFile(h, "test", 4, &n, nullptr); CloseHandle(h); return ok && n == 4;
}
static std::wstring quote(const std::wstring& value) {
    std::wstring result = L"\""; size_t slash = 0;
    for (auto c : value) { if (c == L'\\') { ++slash; continue; } result.append(c == L'"' ? slash * 2 + 1 : slash, L'\\'); slash = 0; result += c; }
    result.append(slash * 2, L'\\'); return result + L'"';
}
static bool inherited_handle_denied(HANDLE handle) {
    __try {
        if (GetFileType(handle) != FILE_TYPE_DISK) return true;
        char byte = 0; DWORD count = 0; return !ReadFile(handle, &byte, 1, &count, nullptr);
    } __except (GetExceptionCode() == 0xC0000008 ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) { return true; }
}
static std::wstring env(const wchar_t* key) {
    std::wstring value(32768, L'\0'); DWORD n = GetEnvironmentVariableW(key, value.data(), static_cast<DWORD>(value.size())); if (!n || n >= value.size()) return L""; value.resize(n); return value;
}
int wmain(int argc, wchar_t** argv) {
    if (!SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)) return 126;
    if (argc == 3 && std::wstring(argv[1]) == L"shell-child") {
        HANDLE token = nullptr; DWORD app = 0, size = 0;
        if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) return 120;
        BOOL queried = GetTokenInformation(token, TokenIsAppContainer, &app, sizeof(app), &size); CloseHandle(token);
        return queried && app && !read(argv[2]) && env(L"GENT_FIXTURE_AMBIENT_SECRET").empty() ? 0 : 121;
    }
    if (argc == 3 && std::wstring(argv[1]) == L"shell-contract") {
        auto shell = env(L"CLAUDE_CODE_GIT_BASH_PATH"), path = env(L"PATH");
        if (shell.empty() || path.rfind(shell.substr(0, shell.find_last_of(L'\\')) + L";", 0) != 0) return 122;
        std::wstring line = quote(shell) + L" shell-child " + quote(argv[2]); STARTUPINFOW startup{}; startup.cb = sizeof(startup); PROCESS_INFORMATION child{};
        if (!CreateProcessW(shell.c_str(), line.data(), nullptr, nullptr, FALSE, CREATE_NO_WINDOW, nullptr, nullptr, &startup, &child)) { std::fprintf(stderr, "shell-child-create:%lu\n", GetLastError()); return 123; }
        DWORD waited = WaitForSingleObject(child.hProcess, 5000), code = 124;
        if (waited == WAIT_OBJECT_0) GetExitCodeProcess(child.hProcess, &code); else TerminateProcess(child.hProcess, 124);
        CloseHandle(child.hThread); CloseHandle(child.hProcess); return static_cast<int>(code);
    }
    int resource = probe_resource_mode(argc, argv); if (resource >= 0) return resource;
    if (argc >= 5 && std::wstring(argv[1]) == L"host-inherited-handle") {
        SECURITY_ATTRIBUTES inherit{sizeof(inherit), nullptr, TRUE};
        HANDLE secret = CreateFileW(argv[2], GENERIC_READ, FILE_SHARE_READ, &inherit, OPEN_EXISTING, 0, nullptr); if (secret == INVALID_HANDLE_VALUE) return 110;
        std::wstring line = quote(argv[3]); for (int i = 4; i < argc; ++i) line += L" " + quote(argv[i]); line += L" " + std::to_wstring(reinterpret_cast<uintptr_t>(secret));
        STARTUPINFOW startup{}; startup.cb = sizeof(startup); startup.dwFlags = STARTF_USESTDHANDLES;
        startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE); startup.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE); startup.hStdError = GetStdHandle(STD_ERROR_HANDLE);
        for (auto handle : {startup.hStdInput, startup.hStdOutput, startup.hStdError}) SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
        PROCESS_INFORMATION pi{}; if (!CreateProcessW(argv[3], line.data(), nullptr, nullptr, TRUE, CREATE_NO_WINDOW, nullptr, nullptr, &startup, &pi)) { CloseHandle(secret); return 111; }
        CloseHandle(pi.hThread); WaitForSingleObject(pi.hProcess, INFINITE); DWORD code = 112; GetExitCodeProcess(pi.hProcess, &code); CloseHandle(pi.hProcess); CloseHandle(secret); return static_cast<int>(code);
    }
    if (argc == 2 && std::wstring(argv[1]) == L"same-container-listener") {
        WSADATA data{}; if (WSAStartup(MAKEWORD(2, 2), &data)) return 95;
        SOCKET listener = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP); sockaddr_in address{}; address.sin_family = AF_INET; address.sin_port = htons(45681); address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        if (listener == INVALID_SOCKET || bind(listener, reinterpret_cast<sockaddr*>(&address), sizeof(address)) || listen(listener, 1)) return 96;
        SOCKET connected = accept(listener, nullptr, nullptr); if (connected != INVALID_SOCKET) closesocket(connected); closesocket(listener); WSACleanup(); return connected == INVALID_SOCKET ? 97 : 0;
    }
    if (argc == 3 && std::wstring(argv[1]) == L"descendant") return read(argv[2]) ? 20 : 0;
    if (argc != 10 && argc != 14) return 90;
    bool gateway = argc == 14;
    std::wstring private_root;
    bool private_state = true, private_home = true, private_appdata = true, private_localappdata = true, private_temp = true, private_config = true, local_under_state = true, temp_under_state = true;
    if (gateway) { auto home = env(L"USERPROFILE"); auto state = home.substr(0, home.find_last_of(L"\\/"));
        private_root = state;
        private_home = home == state + L"\\home" && env(L"HOME") == home; private_appdata = env(L"APPDATA") == state + L"\\appdata"; private_localappdata = env(L"LOCALAPPDATA") == state + L"\\localappdata";
        private_temp = env(L"TEMP") == state + L"\\tmp" && env(L"TMP") == state + L"\\tmp"; private_config = env(L"CLAUDE_CONFIG_DIR") == state + L"\\claude" && env(L"CODEX_HOME") == state + L"\\codex";
        local_under_state = env(L"LOCALAPPDATA").rfind(state + L"\\", 0) == 0; temp_under_state = env(L"TEMP").rfind(state + L"\\", 0) == 0 && env(L"TMP").rfind(state + L"\\", 0) == 0;
        private_state = private_home && private_appdata && local_under_state && temp_under_state && private_config; }
    bool ambient_secret_denied = env(L"GENT_FIXTURE_AMBIENT_SECRET").empty();
    bool proxy_runtime_denied = true;
    if (gateway) { auto runtime = std::wstring(argv[11]); auto image = runtime + L"\\gateway.exe", receipt = runtime + L"\\.ownership";
        auto moved = private_root + L"\\renamed-proxy", linked = private_root + L"\\tmp\\proxy-link.exe";
        proxy_runtime_denied = !read(image.c_str()) && !write(receipt.c_str()) && !MoveFileExW(runtime.c_str(), moved.c_str(), 0) && !CreateHardLinkW(linked.c_str(), image.c_str(), nullptr); }
    bool inbound_bound = gateway && begin_inbound_probe(private_root);
    HANDLE leaked = reinterpret_cast<HANDLE>(static_cast<uintptr_t>(std::stoull(argv[argc - 1])));
    bool inherited_denied = inherited_handle_denied(leaked);
    std::wstring self_listener_line = L"\"" + std::wstring(argv[8]) + L"\" same-container-listener";
    STARTUPINFOW listener_startup{}; listener_startup.cb = sizeof(listener_startup); PROCESS_INFORMATION listener_process{};
    bool self_listener = CreateProcessW(argv[8], self_listener_line.data(), nullptr, nullptr, FALSE, CREATE_NO_WINDOW, nullptr, nullptr, &listener_startup, &listener_process) != 0;
    Sleep(100);
    int same_container_error = self_listener ? net(45681) : -1;
    if (self_listener) { if (WaitForSingleObject(listener_process.hProcess, 1000) != WAIT_OBJECT_0) TerminateProcess(listener_process.hProcess, 98); CloseHandle(listener_process.hProcess); CloseHandle(listener_process.hThread); }
    bool allowed_read = read(argv[1]), allowed_write = write(argv[2]);
    HANDLE token = nullptr; DWORD app = 0, token_size = 0;
    if (OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) {
        GetTokenInformation(token, TokenIsAppContainer, &app, sizeof(app), &token_size); CloseHandle(token);
    }
    bool denied_read = !read(argv[3]), denied_write = !write(argv[4]);
    std::wstring hardlink = std::wstring(argv[2]).substr(0, std::wstring(argv[2]).find_last_of(L"\\/")) + L"\\hardlink-escape.txt";
    bool hardlink_denied = !CreateHardLinkW(hardlink.c_str(), argv[3], nullptr);
    bool reparse_denied = !read(argv[5]), readonly_denied = !write(argv[6]);
    uint16_t proxy_port = gateway ? gateway_port() : 0;
    int network_error = net(gateway ? proxy_port : 45679);
    bool http_passed = !gateway || gateway_http(proxy_port);
    bool gateway_allowed = !gateway || network_error == 0;
    int other_port_error = gateway ? net(static_cast<uint16_t>(std::stoul(argv[10]))) : WSAEACCES;
    int ipv6_error = gateway ? net(static_cast<uint16_t>(std::stoul(argv[9])), true) : WSAEACCES;
    int public_error = gateway ? net(443, false, argv[12]) : WSAEACCES;
    bool other_port_denied = other_port_error == WSAEACCES;
    bool ipv6_denied = ipv6_error == WSAEACCES;
    std::wstring line = L"\"" + std::wstring(argv[8]) + L"\" descendant \"" + argv[3] + L"\"";
    STARTUPINFOW startup{}; startup.cb = sizeof(startup); PROCESS_INFORMATION pi{};
    bool descendant = CreateProcessW(argv[8], line.data(), nullptr, nullptr, FALSE, CREATE_NO_WINDOW, nullptr, nullptr, &startup, &pi) != 0;
    DWORD descendant_code = 99;
    if (descendant) { WaitForSingleObject(pi.hProcess, 5000); GetExitCodeProcess(pi.hProcess, &descendant_code); CloseHandle(pi.hThread); CloseHandle(pi.hProcess); }
    std::string report = "{\"appContainer\":" + std::to_string(app) + ",\"sameContainerError\":" + std::to_string(same_container_error) + ",\"allowedRead\":" + std::to_string(allowed_read) + ",\"allowedWrite\":" + std::to_string(allowed_write)
        + ",\"deniedRead\":" + std::to_string(denied_read) + ",\"deniedWrite\":" + std::to_string(denied_write)
        + ",\"hardlinkDenied\":" + std::to_string(hardlink_denied) + ",\"inheritedHandleDenied\":" + std::to_string(inherited_denied)
        + ",\"privateState\":" + std::to_string(private_state) + ",\"ambientSecretDenied\":" + std::to_string(ambient_secret_denied)
        + ",\"privateHome\":" + std::to_string(private_home) + ",\"privateAppdata\":" + std::to_string(private_appdata) + ",\"privateLocalAppdata\":" + std::to_string(private_localappdata)
        + ",\"privateTemp\":" + std::to_string(private_temp) + ",\"privateConfig\":" + std::to_string(private_config)
        + ",\"localAppdataUnderState\":" + std::to_string(local_under_state) + ",\"tempUnderState\":" + std::to_string(temp_under_state)
        + ",\"inboundListenerBound\":" + std::to_string(inbound_bound)
        + ",\"proxyRuntimeDenied\":" + std::to_string(proxy_runtime_denied)
        + ",\"reparseDenied\":" + std::to_string(reparse_denied) + ",\"readonlyWriteDenied\":" + std::to_string(readonly_denied)
        + ",\"networkError\":" + std::to_string(network_error) + ",\"gatewayAllowed\":" + std::to_string(gateway_allowed)
        + ",\"gatewayHttp\":" + std::to_string(http_passed)
        + ",\"otherPortDenied\":" + std::to_string(other_port_denied) + ",\"ipv6Denied\":" + std::to_string(ipv6_denied)
        + ",\"otherPortError\":" + std::to_string(other_port_error) + ",\"ipv6Error\":" + std::to_string(ipv6_error)
        + ",\"publicDirectError\":" + std::to_string(public_error)
        + ",\"descendantDenied\":" + std::to_string(descendant && descendant_code == 0) + "}";
    HANDLE file = CreateFileW(argv[7], GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, 0, nullptr); DWORD n = 0;
    if (file == INVALID_HANDLE_VALUE || !WriteFile(file, report.data(), static_cast<DWORD>(report.size()), &n, nullptr)) return 91;
    CloseHandle(file);
    constexpr std::array<unsigned char, 8> bytes{0, 10, 13, 27, 127, 195, 169, 255};
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), bytes.data(), static_cast<DWORD>(bytes.size()), &n, nullptr);
    WriteFile(GetStdHandle(STD_ERROR_HANDLE), bytes.data(), static_cast<DWORD>(bytes.size()), &n, nullptr);
    std::array<char, 8> input{}; DWORD received = 0;
    while (received < input.size()) { DWORD count = 0; if (!ReadFile(GetStdHandle(STD_INPUT_HANDLE), input.data() + received, static_cast<DWORD>(input.size()) - received, &count, nullptr) || !count) break; received += count; }
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), input.data(), received, &n, nullptr);
    if (gateway) finish_inbound_probe();
    return allowed_read && allowed_write && denied_read && denied_write && hardlink_denied && inherited_denied && private_state && ambient_secret_denied && proxy_runtime_denied && reparse_denied && readonly_denied && network_error == (gateway ? 0 : WSAEACCES)
        && gateway_allowed && http_passed && other_port_denied && ipv6_denied && public_error == WSAEACCES && descendant && descendant_code == 0 && received == 8 ? 0 : 92;
}
