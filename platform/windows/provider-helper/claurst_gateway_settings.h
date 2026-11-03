#pragma once
#include <string>
#include <stdexcept>
namespace claurst_gateway {
inline std::string project(std::string contents, unsigned short port) {
    constexpr char marker[] = "http://127.0.0.1:0/__gent_claurst_proxy_v1__/v1";
    const auto position = contents.find(marker);
    if (!port || position == std::string::npos ||
        contents.find("__gent_claurst_proxy_v1__") != position + 19 ||
        contents.find("__gent_claurst_proxy_v1__", position + sizeof(marker) - 1) != std::string::npos ||
        !position || contents[position - 1] != '"' ||
        contents.size() <= position + sizeof(marker) - 1 || contents[position + sizeof(marker) - 1] != '"')
        throw std::runtime_error("claurst-gateway-settings-template");
    contents.replace(position, sizeof(marker) - 1, "http://127.0.0.1:" + std::to_string(port) + "/v1");
    return contents;
}
}
#ifndef GENT_CLAURST_SETTINGS_PROJECT_ONLY
static void project_claurst_gateway_settings(const std::wstring& path, unsigned short port) {
    Handle file(CreateFileW(path.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr,
        OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr));
    check(file.h != INVALID_HANDLE_VALUE, "claurst-gateway-settings-open");
    BY_HANDLE_FILE_INFORMATION info{}; LARGE_INTEGER size{};
    check(GetFileInformationByHandle(file.h, &info) && GetFileSizeEx(file.h, &size) &&
        !(info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY)) &&
        info.nNumberOfLinks == 1 && size.QuadPart > 0 && size.QuadPart <= 4 * 1024 * 1024,
        "claurst-gateway-settings-file");
    std::string contents(static_cast<size_t>(size.QuadPart), '\0'); DWORD count = 0;
    check(ReadFile(file.h, contents.data(), static_cast<DWORD>(contents.size()), &count, nullptr) &&
        count == contents.size(), "claurst-gateway-settings-read");
    contents = claurst_gateway::project(std::move(contents), port);
    LARGE_INTEGER zero{};
    check(SetFilePointerEx(file.h, zero, nullptr, FILE_BEGIN), "claurst-gateway-settings-seek");
    check(WriteFile(file.h, contents.data(), static_cast<DWORD>(contents.size()), &count, nullptr) &&
        count == contents.size() && SetEndOfFile(file.h) && FlushFileBuffers(file.h),
        "claurst-gateway-settings-write");
}
#endif
