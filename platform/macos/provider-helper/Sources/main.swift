import Foundation

private let helperBundleID = "io.gent.provider-helper"
private let helperVersion = "0.1.0"
private let protocolVersion = 1
private let maximumInputBytes = 32 * 1024

private struct HelperIdentity: Codable {
    let bundleId: String
    let version: String
}

private struct ProviderLock: Codable {
    let name: String
    let canonicalPath: String
    let fileIdentity: String
    let digestSha256: String
    let version: String
    let compatibilityEntry: String
}

private struct Network: Codable {
    let mode: String
    let egressPolicyDigestSha256: String?
}

private struct Limits: Codable {
    let maxProcesses: Int
    let maxMemoryBytes: UInt64
    let maxCpuTimeMs: UInt64
}

private struct LaunchProfile: Codable {
    let workspaceBookmark: String?
    let profileDigestSha256: String
    let network: Network
    let limits: Limits
}

private struct PrepareRequest: Codable {
    let protocolVersion: Int
    let requestId: String
    let operation: String
    let helper: HelperIdentity
    let provider: ProviderLock
    let profile: LaunchProfile
}

private struct ResultValue: Codable {
    let state: String
    let reason: String
}

private struct PrepareResponse: Codable {
    let protocolVersion: Int
    let requestId: String
    let helper: HelperIdentity
    let result: ResultValue
}

private enum ProtocolFailure: Error {
    case invalidRequest(String)
    case helperIdentity
}

private func usage() {
    print("usage: GentProviderHelper [--version|--protocol]")
}

private func response(_ requestId: String, _ state: String, _ reason: String) {
    let value = PrepareResponse(
        protocolVersion: protocolVersion,
        requestId: requestId,
        helper: HelperIdentity(bundleId: helperBundleID, version: helperVersion),
        result: ResultValue(state: state, reason: reason)
    )
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    guard let data = try? encoder.encode(value), let text = String(data: data, encoding: .utf8) else {
        exit(70)
    }
    print(text)
}

private func boundedInput() throws -> Data {
    var input = Data()
    while true {
        let chunk = try FileHandle.standardInput.read(upToCount: 4096) ?? Data()
        if chunk.isEmpty { return input }
        input.append(chunk)
        if input.count > maximumInputBytes { throw ProtocolFailure.invalidRequest("requestTooLarge") }
    }
}

private func exactKeys(_ value: Any, _ expected: Set<String>) throws {
    guard let object = value as? [String: Any], Set(object.keys) == expected else {
        throw ProtocolFailure.invalidRequest("invalidShape")
    }
}

private func decodeRequest(_ data: Data) throws -> PrepareRequest {
    let value = try JSONSerialization.jsonObject(with: data)
    guard let object = value as? [String: Any] else { throw ProtocolFailure.invalidRequest("invalidShape") }
    try exactKeys(object, ["protocolVersion", "requestId", "operation", "helper", "provider", "profile"])
    try exactKeys(object["helper"] as Any, ["bundleId", "version"])
    try exactKeys(object["provider"] as Any, ["name", "canonicalPath", "fileIdentity", "digestSha256", "version", "compatibilityEntry"])
    guard let profile = object["profile"] as? [String: Any] else { throw ProtocolFailure.invalidRequest("invalidShape") }
    let profileKeys = Set(profile.keys)
    guard profileKeys == ["workspaceBookmark", "profileDigestSha256", "network", "limits"] ||
        profileKeys == ["profileDigestSha256", "network", "limits"] else { throw ProtocolFailure.invalidRequest("invalidShape") }
    try exactKeys(profile["network"] as Any, ["mode", "egressPolicyDigestSha256"])
    try exactKeys(profile["limits"] as Any, ["maxProcesses", "maxMemoryBytes", "maxCpuTimeMs"])
    return try JSONDecoder().decode(PrepareRequest.self, from: data)
}

private func digest(_ value: String) -> Bool {
    value.count == 64 && value.utf8.allSatisfy { ($0 >= 48 && $0 <= 57) || ($0 >= 97 && $0 <= 102) }
}

private func label(_ value: String, maximum: Int = 160) -> Bool {
    !value.isEmpty && value.utf8.count <= maximum && !value.contains("\0")
}

private func canonicalPath(_ value: String) -> Bool {
    value.utf8.count <= 1024 && value.hasPrefix("/") && !value.contains("\0") &&
        !value.split(separator: "/").contains("..")
}

private func validate(_ request: PrepareRequest) throws {
    guard request.protocolVersion == protocolVersion, request.operation == "prepare", label(request.requestId, maximum: 96) else {
        throw ProtocolFailure.invalidRequest("unsupportedRequest")
    }
    guard request.helper.bundleId == helperBundleID, request.helper.version == helperVersion else {
        throw ProtocolFailure.invalidRequest("helperIdentityMismatch")
    }
    let provider = request.provider
    guard ["claude", "codex"].contains(provider.name), canonicalPath(provider.canonicalPath),
        label(provider.fileIdentity), label(provider.version), label(provider.compatibilityEntry), digest(provider.digestSha256) else {
        throw ProtocolFailure.invalidRequest("invalidProviderLock")
    }
    let profile = request.profile
    guard digest(profile.profileDigestSha256), profile.limits.maxProcesses > 0,
        profile.limits.maxProcesses <= 1024, profile.limits.maxMemoryBytes > 0,
        profile.limits.maxCpuTimeMs > 0, profile.limits.maxCpuTimeMs <= 604_800_000 else {
        throw ProtocolFailure.invalidRequest("invalidProfile")
    }
    let network = profile.network
    guard ["disabled", "reviewedEgress"].contains(network.mode) else { throw ProtocolFailure.invalidRequest("invalidProfile") }
    if network.mode == "disabled" && network.egressPolicyDigestSha256 != nil { throw ProtocolFailure.invalidRequest("invalidProfile") }
    if network.mode == "reviewedEgress" && !(network.egressPolicyDigestSha256.map(digest) ?? false) { throw ProtocolFailure.invalidRequest("invalidProfile") }
}

private func runtimeIdentityIsExact() -> Bool {
    Bundle.main.bundleIdentifier == helperBundleID &&
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String == helperVersion
}

private func workspaceAuthorization(_ bookmark: String?) -> String {
    guard let bookmark else { return "workspaceBookmarkRequired" }
    guard bookmark.utf8.count <= 8192, let data = Data(base64Encoded: bookmark) else { return "workspaceBookmarkInvalid" }
    var stale = false
    guard let url = try? URL(
        resolvingBookmarkData: data,
        options: [.withSecurityScope, .withoutUI],
        relativeTo: nil,
        bookmarkDataIsStale: &stale
    ), !stale, url.path.hasPrefix("/"), url.startAccessingSecurityScopedResource() else {
        return "workspaceAuthorizationDenied"
    }
    url.stopAccessingSecurityScopedResource()
    return "containmentSemanticsUnavailable"
}

private func runProtocol() {
    var requestId = "unknown"
    do {
        guard runtimeIdentityIsExact() else { throw ProtocolFailure.helperIdentity }
        let request = try decodeRequest(boundedInput())
        requestId = request.requestId
        try validate(request)
        response(request.requestId, "denied", workspaceAuthorization(request.profile.workspaceBookmark))
    } catch ProtocolFailure.invalidRequest(let reason) {
        response(requestId, "invalidRequest", reason)
        exit(65)
    } catch ProtocolFailure.helperIdentity {
        response(requestId, "denied", "helperIdentityInvalid")
        exit(69)
    } catch {
        response(requestId, "invalidRequest", "invalidRequest")
        exit(65)
    }
}

switch Array(CommandLine.arguments.dropFirst()) {
case ["--version"]:
    print("GentProviderHelper \(helperVersion)")
case ["--protocol"]:
    runProtocol()
case []:
    usage()
default:
    fputs("GentProviderHelper: unsupported command\n", stderr)
    usage()
    exit(64)
}
