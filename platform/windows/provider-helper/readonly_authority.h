#pragma once
#include <windows.h>
#include <string>
#include <vector>
void validate_readonly_authority(HANDLE primary_token, const std::vector<std::wstring>& readable, const std::vector<std::wstring>& writable);
