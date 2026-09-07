# Changelog

## Unreleased

### Added

- Configurable minimum estimated savings threshold in Settings; default is 1%.
- Estimated post-compaction size and additional savings during analysis.
- Windows x64 MSVC release CI on current stable Rust, with a downloadable `Compactor-x64` Actions artifact.

### Changed

- Default compression algorithm is now LZX.
- Analysis and compression now use the same sampled compressibility threshold logic.
- Removed blanket file-extension exclusions; archives, media, game containers, and other files are judged by sampled contents instead of filename.
- Default exclusions are limited to Windows, System Volume Information, and Windows-managed root `$*` paths.
- Exclusion matching is case-insensitive and ignores blank/whitespace-only entries.
- Compression progress is byte-aware rather than relying only on file count.
- Incompressible-cache keys now include path, file size, and modification time so changed files are automatically reconsidered.
- The cache backing file is now `incompressible-v2.dat` to avoid stale path-only entries.
- Estimated-savings output has a dedicated legend marker in the classic GUI.

### Fixed

- Correctly treat `DeviceIoControl` as returning a Win32 `BOOL` rather than an HRESULT.
- Correctly surface Win32 errors while handling `ERROR_COMPRESSION_NOT_BENEFICIAL` as a normal no-benefit result.
- Open WOF targets with the read-data/write-attributes access required by the operation.
- Skip encrypted, sparse, offline, reparse-point, and NTFS-compressed special files before WOF classification.
- Use zero-safe progress calculations and saturating size arithmetic to avoid invalid progress or unsigned underflow/overflow edge cases.
- Preserve clear stopped/cancelled state handling.

### Maintenance

- Updated GitHub Actions to current `actions/checkout` and `actions/upload-artifact` majors.
- Removed deprecated Cargo config naming and legacy WinAPI struct-macro warnings.
- Removed unused background helper code.
- Updated project documentation and package metadata for this maintained fork while preserving upstream attribution.

## [0.10.1] - 2020-12-22

### Fixed

- Avoid high CPU usage in GUI loop ([#42])

## [0.10.0] - 2020-12-19

### Changed

- Update dependencies
- Minor UI tweaks due to changes in DPI handling
- Migrate to new dialog crates
- More small internal improvements by @Dr-Emann, thanks! ([#30], [#32])

### Fixed

- Exclusively lock files prior to compaction (should fix [#40], thanks @A-H-M)

## [0.9.0] - 2020-03-03

### Added

- Preserve file timestamps following compression/decompression ([#16])

## [0.8.0] - 2020-02-29

### Added

- Excluded directories now get skipped entirely ([#8])

### Changed

- Paused jobs no longer poll ([#10], @Dr-Emann)
- Less refcounting ([#9], @Dr-Emann)

### Fixed

- Tests ([#11], @Dr-Emann)

### Removed

- WofUtil.dll version check ([#6])

## [0.7.1] - 2019-07-17

### Added

- Initial release

[0.7.1]: https://github.com/Freaky/Compactor/releases/tag/v0.7.1
[0.8.0]: https://github.com/Freaky/Compactor/releases/tag/v0.8.0
[0.9.0]: https://github.com/Freaky/Compactor/releases/tag/v0.9.0
[0.10.0]: https://github.com/Freaky/Compactor/releases/tag/v0.10.0
[0.10.1]: https://github.com/Freaky/Compactor/releases/tag/v0.10.1
[#6]: https://github.com/Freaky/Compactor/issues/6
[#8]: https://github.com/Freaky/Compactor/issues/8
[#9]: https://github.com/Freaky/Compactor/pull/9
[#10]: https://github.com/Freaky/Compactor/pull/10
[#11]: https://github.com/Freaky/Compactor/pull/11
[#16]: https://github.com/Freaky/Compactor/issues/16
[#30]: https://github.com/Freaky/Compactor/pull/30
[#32]: https://github.com/Freaky/Compactor/pull/32
[#40]: https://github.com/Freaky/Compactor/issues/40
[#42]: https://github.com/Freaky/Compactor/issues/42
