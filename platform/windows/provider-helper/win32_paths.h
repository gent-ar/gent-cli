#pragma once
#include <string>
#include <stdexcept>
#include <utility>
namespace gent_windows_paths {
inline std::wstring comparison_path(std::wstring value) {
    if (value.rfind(L"\\\\?\\UNC\\", 0) == 0) value = L"\\\\" + value.substr(8);
    else if (value.rfind(L"\\\\?\\", 0) == 0) value.erase(0, 4);
    for (auto& c : value) if (c == L'/') c = L'\\';
    while (value.size() > 3 && value.back() == L'\\') value.pop_back();
    return value;
}
inline std::wstring io_path(const std::wstring& path) {
    if (path.rfind(L"\\\\?\\", 0) == 0) return path;
    if (path.size() >= 3 && path[1] == L':' && path[2] == L'\\') return L"\\\\?\\" + path;
    if (path.rfind(L"\\\\", 0) == 0) return L"\\\\?\\UNC\\" + path.substr(2);
    throw std::runtime_error("alias-absolute-path");
}
}
