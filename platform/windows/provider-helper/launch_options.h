#pragma once
// Included after the shared native binding helpers in main.cpp.
struct Options {
    std::wstring workspace, exe, sha256, profile, run, state_root, shell, shell_sha256, powershell, powershell_sha256, powershell_launcher, powershell_launcher_sha256;
    std::wstring gateway_api, gateway_url, gateway_token;
    bool gateway_token_stdin = false, read_stdin = false, cpu_uncapped = false;
    uint16_t gateway_port = 0;
    std::wstring gateway_peer;
    std::wstring peer_nonce;
    uint16_t proxy_port = 0;
    std::vector<std::wstring> read, write, env, args;
    uint64_t memory = 0, cpu = 0; DWORD processes = 0;
};
static Options parse(int argc, wchar_t** argv) {
    Options o;
    for (int i = 2; i < argc; ++i) { std::wstring key = argv[i]; if (key == L"--") { while (++i < argc) o.args.emplace_back(argv[i]); break; }
        check(i + 1 < argc, "missing-option"); std::wstring value = argv[++i];
        if (key == L"--workspace") o.workspace = value; else if (key == L"--exe") o.exe = value;
        else if (key == L"--state-root") o.state_root = value;
        else if (key == L"--read-stdin") { check(value == L"1", "readonly-bootstrap-mode"); o.read_stdin = true; }
        else if (key == L"--shell-exe") o.shell = value; else if (key == L"--shell-sha256") o.shell_sha256 = value;
        else if (key == L"--powershell-launcher-exe") o.powershell_launcher = value; else if (key == L"--powershell-launcher-sha256") o.powershell_launcher_sha256 = value;
        else if (key == L"--powershell-exe") o.powershell = value; else if (key == L"--powershell-sha256") o.powershell_sha256 = value;
        else if (key == L"--peer-nonce") o.peer_nonce = value;
        else if (key == L"--sha256") o.sha256 = value; else if (key == L"--profile") o.profile = value; else if (key == L"--run") o.run = value;
        else if (key == L"--read") o.read.push_back(value); else if (key == L"--write") o.write.push_back(value); else if (key == L"--env") o.env.push_back(value);
        else if (key == L"--gateway-api") o.gateway_api = value; else if (key == L"--gateway-url") o.gateway_url = value; else if (key == L"--gateway-token-stdin") { check(value == L"1", "gateway-bootstrap-mode"); o.gateway_token_stdin = true; }
        else if (key == L"--allow-loopback-port") { auto port = std::stoul(value); check(port && port <= 65535, "gateway-port"); o.gateway_port = static_cast<uint16_t>(port); }
        else if (key == L"--cpu-uncapped") { check(value == L"1", "cpu-uncapped-mode"); o.cpu_uncapped = true; }
        else if (key == L"--memory") o.memory = std::stoull(value); else if (key == L"--cpu-ms") o.cpu = std::stoull(value);
        else if (key == L"--processes") { auto n = std::stoull(value); check(n <= 65535, "process-limit"); o.processes = static_cast<DWORD>(n); } else fail("unknown-option"); }
    check(!o.workspace.empty() && !o.exe.empty() && digest_string(o.sha256) && digest_string(o.profile), "binding");
    check(!o.run.empty() && o.run.size() <= 128 && std::all_of(o.run.begin(), o.run.end(), [](wchar_t c) { return (c >= L'a' && c <= L'z') || (c >= L'A' && c <= L'Z') || (c >= L'0' && c <= L'9') || c == L'-' || c == L'_'; }), "run-id");
    check(o.memory && o.memory <= SIZE_MAX && o.cpu && o.cpu < static_cast<uint64_t>(INT64_MAX / 10000) && o.processes, "resource-limit");
    if (o.gateway_token_stdin) {
        unsigned char frame[72]{}; DWORD total = 0;
        while (total < sizeof(frame)) {
            DWORD got = 0;
            if (!ReadFile(GetStdHandle(STD_INPUT_HANDLE), frame + total, static_cast<DWORD>(sizeof(frame)) - total, &got, nullptr) || !got) {
                SecureZeroMemory(frame, sizeof(frame)); fail("gateway-bootstrap-read");
            }
            total += got;
        }
        const bool valid_magic = std::memcmp(frame, "GENTGW01", 8) == 0;
        if (valid_magic) for (size_t i = 8; i < sizeof(frame); ++i) o.gateway_token.push_back(static_cast<wchar_t>(frame[i]));
        SecureZeroMemory(frame, sizeof(frame));
        check(valid_magic && digest_string(o.gateway_token), "gateway-bootstrap-binding");
    }
    if (o.read_stdin) {
        check(!o.powershell.empty() && digest_string(o.powershell_sha256), "readonly-bootstrap-powershell");
        read_readonly_bootstrap(GetStdHandle(STD_INPUT_HANDLE),o.read);
    }
    check((o.shell.empty() && o.shell_sha256.empty()) || (!o.shell.empty() && digest_string(o.shell_sha256)), "shell-binding");
    if (!o.shell.empty()) {
        auto shell = normalized_path(o.shell); bool granted = false;
        for (const auto& root : o.read) { auto read = normalized_path(root); if (shell.size() >= read.size() && _wcsnicmp(shell.c_str(), read.c_str(), read.size()) == 0 && (shell.size() == read.size() || shell[read.size()] == L'\\')) granted = true; }
        check(granted && !(GetFileAttributesW(o.shell.c_str()) & FILE_ATTRIBUTE_DIRECTORY), "shell-read-grant");
        for (const auto& root : o.write) { auto write = normalized_path(root); check(!(shell.size() >= write.size() && _wcsnicmp(shell.c_str(), write.c_str(), write.size()) == 0 && (shell.size() == write.size() || shell[write.size()] == L'\\')), "shell-writable-grant"); }
    }
    check((o.powershell.empty() && o.powershell_sha256.empty()) || (!o.powershell.empty() && digest_string(o.powershell_sha256)), "powershell-binding");
    if (!o.powershell.empty()) {
        auto ps = normalized_path(o.powershell); bool granted = false;
        auto basename = ps.substr(ps.find_last_of(L'\\') + 1);
        check(_wcsicmp(basename.c_str(), L"pwsh.exe") == 0, "powershell-basename");
        for (const auto& root : o.read) { auto read = normalized_path(root); if (ps.size() >= read.size() && _wcsnicmp(ps.c_str(), read.c_str(), read.size()) == 0 && (ps.size() == read.size() || ps[read.size()] == L'\\')) granted = true; }
        const DWORD attrs = GetFileAttributesW(o.powershell.c_str());
        check(granted && attrs != INVALID_FILE_ATTRIBUTES && !(attrs & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)), "powershell-read-grant");
        for (const auto& root : o.write) { auto write = normalized_path(root); check(!(ps.size() >= write.size() && _wcsnicmp(ps.c_str(), write.c_str(), write.size()) == 0 && (ps.size() == write.size() || ps[write.size()] == L'\\')), "powershell-writable-grant"); }
    }
    check((o.powershell_launcher.empty() && o.powershell_launcher_sha256.empty()) || (!o.powershell_launcher.empty() && !o.powershell.empty() && digest_string(o.powershell_launcher_sha256)), "powershell-launcher-binding");
    if (!o.powershell_launcher.empty()) {
        auto launcher = normalized_path(o.powershell_launcher); bool granted = false;
        check(_wcsicmp(launcher.substr(launcher.find_last_of(L'\\') + 1).c_str(), L"pwsh.exe") == 0, "powershell-launcher-basename");
        for (const auto& root : o.read) { auto read = normalized_path(root); if (launcher.size() >= read.size() && _wcsnicmp(launcher.c_str(), read.c_str(), read.size()) == 0 && (launcher.size() == read.size() || launcher[read.size()] == L'\\')) granted = true; }
        const DWORD attrs = GetFileAttributesW(o.powershell_launcher.c_str());
        check(granted && attrs != INVALID_FILE_ATTRIBUTES && !(attrs & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)), "powershell-launcher-read-grant");
        for (const auto& root : o.write) { auto write = normalized_path(root); check(!(launcher.size() >= write.size() && _wcsnicmp(launcher.c_str(), write.c_str(), write.size()) == 0 && (launcher.size() == write.size() || launcher[write.size()] == L'\\')), "powershell-launcher-writable-grant"); }
    }
    check(!o.cpu_uncapped || (!o.gateway_port && o.shell.empty()), "cpu-uncapped-scope");
    check(!o.gateway_port == o.powershell_launcher.empty() && (!o.gateway_port || !o.shell.empty()), "command-broker-shell-runtime");
    if (o.gateway_port || !o.gateway_api.empty() || !o.gateway_url.empty() || !o.gateway_token.empty()) {
        check(o.gateway_token_stdin && o.gateway_port && (o.gateway_api == L"claude" || o.gateway_api == L"codex" || o.gateway_api == L"claurst") && digest_string(o.gateway_token)
            && o.gateway_url == L"http://127.0.0.1:" + std::to_wstring(o.gateway_port), "gateway-binding");
        check(!o.state_root.empty() && std::find(o.write.begin(), o.write.end(), o.state_root) != o.write.end(), "gateway-private-state");
        auto state = normalized_path(o.state_root), workspace = normalized_path(o.workspace);
        check(!(state.size() >= workspace.size() && _wcsnicmp(state.c_str(), workspace.c_str(), workspace.size()) == 0 && (state.size() == workspace.size() || state[workspace.size()] == L'\\')), "gateway-state-overlaps-workspace");
        check(o.peer_nonce.size() == 32 && digest_string(o.peer_nonce + o.peer_nonce), "gateway-peer-nonce");
    }
    for (const auto& e : o.env) { auto p = e.find(L'='); check(p != std::wstring::npos && p, "environment"); auto key = e.substr(0, p);
        check(key == L"COLORTERM" || key == L"LANG" || key == L"LC_ALL" || key == L"LC_CTYPE" || key == L"NO_COLOR" || key == L"TERM" || key == L"TMP" || key == L"TMPDIR", "environment-key"); }
    return o;
}
