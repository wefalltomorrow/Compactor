# Compactor

A small, native Windows GUI for applying Windows Overlay Filter (WOF) filesystem compression to folders.

This repository is a maintained fork of [Freaky/Compactor](https://github.com/Freaky/Compactor). It keeps the original lightweight Rust GUI and focused workflow, while fixing correctness issues and improving analysis, safety, and build maintenance without turning Compactor into a larger suite.

## Highlights

- Windows WOF compression with **XPRESS4K, XPRESS8K, XPRESS16K, and LZX**
- **LZX is the default** for maximum space savings
- sampled compressibility analysis before files are compacted
- configurable **minimum estimated savings** threshold; default is **1%**
- estimated post-compaction size and additional savings shown during analysis
- pause, resume, and stop support
- preserves file timestamps after compression/decompression
- remembers files that were not worth compressing, while automatically reconsidering them after they change
- case-insensitive user exclusions with blank-rule protection
- skips unsafe/special file types such as encrypted, sparse, offline, reparse-point, and NTFS-compressed files
- no blanket file-extension exclusions: files are judged by sampled contents rather than filename

## Defaults

Compactor intentionally keeps its defaults simple:

- **Compression:** LZX
- **Minimum estimated savings:** 1%
- **Excluded paths:** Windows, System Volume Information, and Windows-managed root `$*` paths such as `$Recycle.Bin`

Archives, media, textures, game containers, and other file types are not excluded merely because of their extension. If a file is already effectively incompressible, the estimator and minimum-savings threshold will reject it.

## How it works

Compactor uses Windows WOF compression through `DeviceIoControl` and the WOF APIs rather than rewriting file contents itself.

Before compacting an eligible file, Compactor samples blocks from the file and passes them through the existing `compresstimator` logic to estimate compressibility. Analysis uses the same threshold logic as the actual compression path.

The estimate is deliberately presented as an **estimate**. Sampling uses LZ4 as a fast compressibility proxy, while the final file is compressed by Windows using the selected WOF algorithm, so the final on-disk size can differ.

## Installation

This fork currently produces a **Windows x64** release build in GitHub Actions.

Until a tagged release is published, download the `Compactor-x64` artifact from the latest successful **Windows x64 build** under the repository's [Actions](https://github.com/wefalltomorrow/Compactor/actions) page. GitHub Actions artifacts require a GitHub sign-in and expire after their retention period.

Compactor is portable: extract/run `Compactor.exe`; there is no installer or background service.

### Build from source

Requirements:

- Windows 10 or Windows 11
- Rust stable with the `x86_64-pc-windows-msvc` toolchain
- Microsoft C++ build tools required by the Rust MSVC toolchain

Build:

```powershell
cargo build --release
```

The executable will be written to:

```text
target\release\Compactor.exe
```

CI builds and tests the same x64 MSVC release target on Windows.

## Usage

1. Choose a folder.
2. Let Compactor analyse its contents.
3. Review current disk usage and the estimated post-compaction savings.
4. Adjust the compression mode or minimum-savings threshold in **Settings** if desired.
5. Click **Compress**.

Use **Decompress** to remove WOF backing from files previously compressed by Compactor.

## Safety and caveats

Compactor is best suited to application and game files that rarely change. Modifying a WOF-compressed file causes Windows to materialise it again, so folders that receive updates may benefit from being re-analysed and re-compressed afterward.

Avoid blindly compacting an entire system drive. The default exclusions protect common Windows-managed locations, but custom folders can still contain databases, virtual machines, active logs, or other write-heavy files that are poor candidates for this type of compression.

Compactor does not elevate itself through UAC. Protected files may require launching the program with appropriate permissions.

As with any tool that changes filesystem metadata, keep backups of important data. Compactor is provided without warranty under the MIT License.

## Differences from upstream v0.10.1

This fork intentionally stays close to the original program while addressing several long-standing issues:

- fixes Win32 `BOOL` handling for WOF `DeviceIoControl` calls
- correctly handles `ERROR_COMPRESSION_NOT_BENEFICIAL`
- uses the required read/write-attributes access for WOF operations
- makes exclusions case-insensitive and ignores blank entries
- skips encrypted, sparse, offline, reparse-point, and NTFS-compressed files before WOF classification
- keys the incompressible cache by path + size + modification time so updated files are retried
- adds zero-safe/overflow-safe size and progress accounting
- makes compression progress byte-aware
- adds configurable savings thresholds and estimated savings to the classic GUI
- defaults to LZX and lets the estimator decide whether file contents are worth compressing
- maintains a current Windows x64 CI build on stable Rust

See [CHANGELOG.md](CHANGELOG.md) for the maintained change history.

## Scope

Compactor is deliberately kept narrow. This fork does **not** add a scheduler, updater, background service, Steam database, telemetry, automatic algorithm switching, CompactOS controls, or default parallel compression. Those features can be useful elsewhere, but they are outside this project's goal of remaining a small, understandable compression GUI.

## Technical notes

The application is primarily written in [Rust](https://www.rust-lang.org/) and uses a local embedded web-view for the GUI. It does not require remote web resources to operate.

WOF compression is applied through [`FSCTL_SET_EXTERNAL_BACKING`](https://learn.microsoft.com/windows-hardware/drivers/ifs/fsctl-set-external-backing) and removed through [`FSCTL_DELETE_EXTERNAL_BACKING`](https://learn.microsoft.com/windows-hardware/drivers/ifs/fsctl-delete-external-backing).

The sampled compressibility estimator comes from Thomas Hurst's [compresstimator](https://github.com/Freaky/compresstimator) project.

## Credits

Compactor was originally written by **Thomas Hurst (Freaky)**. This fork preserves that work and its MIT license while maintaining targeted fixes and improvements at [wefalltomorrow/Compactor](https://github.com/wefalltomorrow/Compactor).

See [LICENSE.txt](LICENSE.txt) for license terms.
