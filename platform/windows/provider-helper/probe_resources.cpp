#include <windows.h>
#include "probe_resources.h"
#include "probe_network.h"
#include <string>
#include <array>
#include <iostream>

static std::wstring quote(const std::wstring& value) {
    std::wstring result = L"\""; size_t slash = 0;
    for (auto c : value) { if (c == L'\\') { ++slash; continue; } result.append(c == L'"' ? slash * 2 + 1 : slash, L'\\'); slash = 0; result += c; }
    result.append(slash * 2, L'\\'); return result + L'"';
}
static bool spawn(const std::wstring& mode, const std::wstring& path, bool wait) {
    std::wstring image(32768, L'\0'); DWORD n = GetModuleFileNameW(nullptr, image.data(), static_cast<DWORD>(image.size())); if (!n || n >= image.size()) return false; image.resize(n);
    std::wstring line = quote(image) + L" " + quote(mode) + L" " + quote(path); STARTUPINFOW startup{}; startup.cb = sizeof(startup); PROCESS_INFORMATION pi{};
    if (!CreateProcessW(image.c_str(), line.data(), nullptr, nullptr, FALSE, CREATE_NO_WINDOW, nullptr, nullptr, &startup, &pi)) return false;
    if (wait) { if (WaitForSingleObject(pi.hProcess, 5000) != WAIT_OBJECT_0) TerminateProcess(pi.hProcess, 114); }
    CloseHandle(pi.hThread); CloseHandle(pi.hProcess); return true;
}
static bool record(const wchar_t* path) {
    FILETIME creation{}, exited{}, kernel{}, user{}; if (!GetProcessTimes(GetCurrentProcess(), &creation, &exited, &kernel, &user)) return false;
    ULARGE_INTEGER time{}; time.LowPart = creation.dwLowDateTime; time.HighPart = creation.dwHighDateTime;
    std::string line = std::to_string(GetCurrentProcessId()) + ":" + std::to_string(time.QuadPart) + "\n";
    HANDLE file = CreateFileW(path, FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_ALWAYS, FILE_FLAG_WRITE_THROUGH, nullptr); if (file == INVALID_HANDLE_VALUE) return false;
    DWORD written = 0; BOOL ok = WriteFile(file, line.data(), static_cast<DWORD>(line.size()), &written, nullptr); CloseHandle(file); return ok && written == line.size();
}
int probe_resource_mode(int argc, wchar_t** argv) {
    if (argc < 2) return -1; std::wstring mode = argv[1];
    if (mode == L"network-check" && argc == 5) {
        auto port = std::stoul(argv[3]); if (!port || port > 65535) return 123;
        int error = net(static_cast<uint16_t>(port), false, argv[2]); bool expected = std::wstring(argv[4]) == L"allowed";
        std::cout << "network-error:" << error << "\n"; return error == (expected ? 0 : 10013) ? 0 : 124;
    }
    if (mode == L"memory-check" && argc == 3) {
        constexpr SIZE_T size = 32 * 1024 * 1024; auto memory = static_cast<volatile unsigned char*>(VirtualAlloc(nullptr, size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE));
        bool allocated = memory != nullptr;
        if (allocated) { for (SIZE_T i = 0; i < size; i += 4096) memory[i] = 1; VirtualFree(const_cast<unsigned char*>(memory), 0, MEM_RELEASE); }
        bool expected = std::wstring(argv[2]) == L"allowed"; std::cout << (allocated ? "memory-allowed\n" : "memory-denied\n"); return allocated == expected ? 0 : 115;
    }
    if (mode == L"child-marker" && argc == 3) return record(argv[2]) ? 0 : 116;
    if (mode == L"process-check" && argc == 4) {
        bool created = spawn(L"child-marker", argv[2], true); bool executed = GetFileAttributesW(argv[2]) != INVALID_FILE_ATTRIBUTES;
        bool expected = std::wstring(argv[3]) == L"allowed"; std::cout << (executed ? "child-executed\n" : "child-denied\n");
        return expected ? (created && executed ? 0 : 117) : (!executed ? 0 : 118);
    }
    if ((mode == L"tree-run" || mode == L"tree-child" || mode == L"tree-leaf") && argc == 3) {
        if (!record(argv[2])) return 119;
        if (mode == L"tree-run" && !spawn(L"tree-child", argv[2], false)) return 120;
        if (mode == L"tree-child" && !spawn(L"tree-leaf", argv[2], false)) return 121;
        Sleep(INFINITE); return 122;
    }
    return -1;
}
