# Windows external proxy release build

The Windows sandbox release requires a reviewed descriptor, the exact helper and finite resources it identifies, its shell closures, and a genuinely signed proxy MSIX. A combined descriptor may cover Codex, Claude and Claurst; Claurst additionally requires its paired model runtime. This build does not certify installed behavior.

`tools/build-windows-sandbox-release.py` binds the existing daemon compile-time pins to those input bytes. It reuses the optional release component verifier, requires a new output directory, checks the copied inputs again, builds all CLI/daemon binaries, and verifies the runtime files after compilation. The inherited environment is copied; pins apply only to the Cargo child. Release profile overrides supplied by the build operator still apply.

```powershell
python tools/build-windows-sandbox-release.py `
  --cargo D:/cargo/bin/cargo.exe `
  --target-dir D:/build/gent-sandbox-target `
  --runtime D:/reviewed/runtime/windows `
  --descriptor-sha256 <reviewed-descriptor-sha256> `
  --proxy-package D:/reviewed/proxy.msix `
  --sign-tool '<Windows-SDK>/x64/signtool.exe' `
  --out D:/build/gent-sandbox-stage
```

`--prepare-only` verifies and stages without invoking Cargo. A failed build retains an unfinished stage; use a new output path for a later attempt. The tool never replaces an existing stage, installs a package, changes certificate trust, edits a pointer, or publishes an artifact.

The new stage contains `bin/{gent.exe,gentd.exe,gent-launcher.exe}`, `runtime/windows/`, and `build-binding.json`. The binding records the four compile-time pins, command, runtime hashes, binary hashes and phase. `phase=built` means the build and copy checks completed; `installedAcceptance` remains false.

Pass `stage/bin` as the normal `package-release.py --target-dir`, and pass these optional arguments alongside its existing Node, Claurst and authority inputs:

```powershell
--windows-sandbox-runtime-dir D:/build/gent-sandbox-stage/runtime/windows `
--windows-sandbox-descriptor-sha256 <same-reviewed-descriptor-sha256> `
--windows-proxy-package D:/build/gent-sandbox-stage/runtime/windows/proxy-package.msix `
  --windows-sign-tool '<Windows-SDK>/x64/signtool.exe' `
  --windows-build-binding D:/build/gent-sandbox-stage/build-binding.json
```

The packager requires a completed build binding and checks its binary hashes, runtime hashes and compile-time pins before writing the archive, then checks the archived binary and sandbox bytes. This prevents accidental cohort mixing; the binding itself is not a signature or independent proof that the build was trustworthy.

For the automated release, repository variables `GENT_WINDOWS_SANDBOX_ARTIFACT_RUN_ID`, `GENT_WINDOWS_SANDBOX_DESCRIPTOR_SHA256`, and `GENT_WINDOWS_MODEL_MANIFEST_SHA256` identify a prior run in the same repository and the independently reviewed descriptor and model manifest digests. That run must retain an artifact named `reviewed-windows-sandbox` containing `runtime/windows/`, its sibling `runtime/claurst/`, and `proxy-package.msix` at the artifact root. The workflow verifies the MSIX with Windows SignTool, binds the backend to the reviewed inputs, checks the sandbox Claurst payload against the independently staged pinned release runtime, and packages the bound binaries. Missing variables or inputs fail the Windows job. Providing those inputs and successfully running this workflow are still required; this wiring does not create a signing identity, sign a proxy, or prove installed behavior.

The proxy image and manifest must be finalized before their helper authority pins; finalize any PE signatures before hashing. Build the helper, then finalize the descriptor's helper/resources/package hashes, then build Gentd with the descriptor pin. Verify the actual signed package payload and manifest against those values. A new package version/full name requires corresponding reviewed authority and a new build.

No production code-signing identity is provisioned on the inspected development PC. The existing Store channel can submit an unsigned candidate for Microsoft certification/signing; its signed result still needs exact payload verification and installed acceptance. A directly distributed MSIX requires a genuinely trusted signer. A loose development registration is not a signed release substitute. Separately authorized isolated QA continues through its explicit development-only build and fixture path without modifying machine trust.

Tests: `python tools/test-build-windows-sandbox-release.py -v`. These test build wiring and drift/failure handling using fixture artifacts and a fixture signature verifier. They exercise a real child process for environment propagation, not Cargo compilation, package cryptographic trust, provider inference, or Windows containment.
