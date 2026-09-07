use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError};
use filesize::PathExt;

use crate::background::BackgroundHandle;
use crate::compression::BackgroundCompactor;
use crate::folder::{worker_count_for_path, FileInfo, FileKind, FolderInfo, FolderScan};
use crate::gui::{GuiRequest, GuiWrapper};
use crate::persistence::{config, incompressible_key, pathdb};

pub struct Backend<T> {
    gui: GuiWrapper<T>,
    msg: Receiver<GuiRequest>,
    info: Option<FolderInfo>,
}

fn format_size(size: u64, decimal: bool) -> String {
    use humansize::{file_size_opts as options, FileSize};

    size.file_size(if decimal {
        options::DECIMAL
    } else {
        options::BINARY
    })
    .expect("file size")
}

fn progress(done_bytes: u64, total_bytes: u64) -> f32 {
    if total_bytes == 0 {
        1.0
    } else {
        (done_bytes as f64 / total_bytes as f64).min(1.0) as f32
    }
}


#[derive(Default)]
struct CompressedFolderTotals {
    count: usize,
    logical_size: u64,
    physical_size: u64,
}

fn build_compressed_report(folder: &FolderInfo, decimal: bool) -> String {
    let summary = folder.compressed.summary();
    let saved = summary.logical_size.saturating_sub(summary.physical_size);
    let mut folders: BTreeMap<PathBuf, CompressedFolderTotals> = BTreeMap::new();
    let mut files: Vec<&FileInfo> = folder.compressed.files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));

    for fi in &files {
        let parent = fi
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let totals = folders.entry(parent).or_default();
        totals.count += 1;
        totals.logical_size = totals.logical_size.saturating_add(fi.logical_size);
        totals.physical_size = totals.physical_size.saturating_add(fi.physical_size);
    }

    let mut report = String::new();
    writeln!(&mut report, "Compactor compressed-file report").unwrap();
    writeln!(&mut report, "Root: {}", folder.path.display()).unwrap();
    writeln!(&mut report).unwrap();
    writeln!(&mut report, "{} compressed files", summary.count).unwrap();
    writeln!(&mut report, "Logical size: {}", format_size(summary.logical_size, decimal)).unwrap();
    writeln!(&mut report, "On-disk size: {}", format_size(summary.physical_size, decimal)).unwrap();
    writeln!(&mut report, "Saved: {}", format_size(saved, decimal)).unwrap();
    writeln!(&mut report).unwrap();
    writeln!(&mut report, "Folders containing compressed files").unwrap();
    writeln!(&mut report, "Count\tLogical\tOn-disk\tSaved\tFolder").unwrap();

    for (path, totals) in folders {
        writeln!(
            &mut report,
            "{}\t{}\t{}\t{}\t{}",
            totals.count,
            format_size(totals.logical_size, decimal),
            format_size(totals.physical_size, decimal),
            format_size(
                totals.logical_size.saturating_sub(totals.physical_size),
                decimal,
            ),
            path.display(),
        )
        .unwrap();
    }

    writeln!(&mut report).unwrap();
    writeln!(&mut report, "Compressed files").unwrap();
    writeln!(&mut report, "Logical\tOn-disk\tSaved\tFile").unwrap();

    for fi in files {
        writeln!(
            &mut report,
            "{}\t{}\t{}\t{}",
            format_size(fi.logical_size, decimal),
            format_size(fi.physical_size, decimal),
            format_size(fi.logical_size.saturating_sub(fi.physical_size), decimal),
            fi.path.display(),
        )
        .unwrap();
    }

    report
}

impl<T> Backend<T> {
    pub fn new(gui: GuiWrapper<T>, msg: Receiver<GuiRequest>) -> Self {
        Self {
            gui,
            msg,
            info: None,
        }
    }

    pub fn run(&mut self) {
        loop {
            match self.msg.recv() {
                Ok(GuiRequest::ChooseFolder) => {
                    let path = self.gui.choose_folder().recv().ok().flatten();

                    if let Some(path) = path {
                        self.gui.folder(&path);
                        self.scan_loop(path);
                    }
                }
                Ok(GuiRequest::Analyse) if self.info.is_some() => {
                    let path = self.info.take().unwrap().path;
                    self.gui.folder(&path);
                    self.scan_loop(path);
                }
                Ok(GuiRequest::ViewCompressed) if self.info.is_some() => {
                    self.view_compressed();
                }
                Ok(GuiRequest::Compress) if self.info.is_some() => {
                    self.compress_loop();
                }
                Ok(GuiRequest::Decompress) if self.info.is_some() => {
                    self.uncompress_loop();
                }
                Ok(msg) => {
                    eprintln!("Backend: Ignored message: {:?}", msg);
                }
                Err(_) => {
                    eprintln!("Backend: exit run loop");
                    break;
                }
            }
        }
    }

    fn scan_loop(&mut self, path: PathBuf) {
        let current = config().read().unwrap().current();
        let excludes = current.globset().expect("globs");
        let ratio_limit = current.ratio_limit();

        let scanner = FolderScan::new(
            path,
            excludes,
            ratio_limit,
            current.max_threads,
            current.hdd_single_thread,
        );
        let task = BackgroundHandle::spawn(scanner);
        let start = Instant::now();

        self.gui.status("Analysing", None);
        loop {
            let msg = self.msg.recv_timeout(Duration::from_millis(25));

            match msg {
                Ok(GuiRequest::Pause) => {
                    task.pause();
                    self.gui.status("Paused", Some(0.5));
                    self.gui.paused();
                }
                Ok(GuiRequest::Resume) => {
                    task.resume();
                    self.gui.status("Analysing", None);
                    self.gui.resumed();
                }
                Ok(GuiRequest::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    task.cancel();
                }
                Ok(msg) => {
                    eprintln!("Ignored message: {:?}", msg);
                }
                Err(RecvTimeoutError::Timeout) => (),
            }

            match task.wait_timeout(Duration::from_millis(25)) {
                Some(Ok(info)) => {
                    self.gui
                        .status(format!("Analysed in {:.2?}", start.elapsed()), Some(1.0));
                    self.gui.summary(info.summary());
                    self.gui.scanned();
                    self.info = Some(info);
                    break;
                }
                Some(Err(info)) => {
                    self.gui.status(
                        format!("Analysis stopped after {:.2?}", start.elapsed()),
                        Some(0.5),
                    );
                    self.gui.summary(info.summary());
                    self.gui.stopped();
                    self.info = Some(info);
                    break;
                }
                None => {
                    if let Some(status) = task.status() {
                        self.gui
                            .status(format!("Analysing: {}", status.0.display()), None);
                        self.gui.summary(status.1);
                    }
                }
            }
        }
    }

    fn view_compressed(&self) {
        let Some(folder) = self.info.as_ref() else {
            return;
        };
        let decimal = config().read().unwrap().current().decimal;
        let report_path = std::env::temp_dir().join("Compactor-compressed-files.txt");

        if let Err(err) = fs::write(&report_path, build_compressed_report(folder, decimal)) {
            self.gui.status(
                format!("Unable to create compressed-file report: {}", err),
                Some(1.0),
            );
            return;
        }

        if let Err(err) = open::that(&report_path) {
            self.gui.status(
                format!("Unable to open compressed-file report: {}", err),
                Some(1.0),
            );
        }
    }

    fn compress_loop(&mut self) {
        let current = config().read().unwrap().current();
        let compression = Some(current.compression);
        let mut folder = self.info.take().expect("fileinfo");
        let worker_count = worker_count_for_path(
            &folder.path,
            current.max_threads,
            current.hdd_single_thread,
        );

        let (send_file, send_file_rx) = bounded::<(PathBuf, u64)>(worker_count);
        let (recv_result_tx, recv_result) =
            bounded::<(PathBuf, io::Result<bool>)>(worker_count);
        let mut tasks = Vec::with_capacity(worker_count);

        for _ in 0..worker_count {
            let compactor = BackgroundCompactor::new(
                compression,
                current.ratio_limit(),
                send_file_rx.clone(),
                recv_result_tx.clone(),
            );
            tasks.push(BackgroundHandle::spawn(compactor));
        }

        drop(send_file_rx);
        drop(recv_result_tx);

        let start = Instant::now();
        let summary = folder.summary();
        let total_files = folder.len(FileKind::Compressible);
        let total_bytes = summary.compressible.logical_size;
        let compressible_size = summary.compressible.physical_size;
        let mut done_files = 0usize;
        let mut done_bytes = 0u64;
        let mut pending: HashMap<PathBuf, FileInfo> = HashMap::with_capacity(worker_count);
        let mut no_more_files = false;

        let mut last_update = Instant::now();
        let mut last_write = Instant::now();
        let mut paused = false;
        let mut stopped = false;

        let old_size = folder.physical_size;

        let incompressible = pathdb();
        let mut incompressible = incompressible.write().unwrap();
        let _ = incompressible.load();

        self.gui.compacting();
        self.gui.status("Compacting", Some(0.0));

        loop {
            while !paused && !stopped && pending.len() < worker_count && !no_more_files {
                if let Some(fi) = folder.pop(FileKind::Compressible) {
                    let path = folder.path.join(&fi.path);
                    let logical_size = fi.logical_size;

                    if send_file.send((path.clone(), logical_size)).is_err() {
                        folder.push(FileKind::Compressible, fi);
                        stopped = true;
                        break;
                    }

                    pending.insert(path, fi);
                } else {
                    no_more_files = true;
                }
            }

            if (no_more_files || stopped) && pending.is_empty() {
                break;
            }

            if last_write.elapsed() > Duration::from_secs(60) {
                let _ = incompressible.save();
                last_write = Instant::now();
            }

            loop {
                match self.msg.try_recv() {
                    Ok(GuiRequest::Pause) if !paused && !stopped => {
                        paused = true;
                        self.gui.paused();
                        self.gui.status(
                            if pending.is_empty() {
                                "Paused"
                            } else {
                                "Pausing after active files finish"
                            },
                            Some(progress(done_bytes, total_bytes)),
                        );
                    }
                    Ok(GuiRequest::Resume) if paused && !stopped => {
                        paused = false;
                        self.gui.resumed();
                        self.gui.status(
                            "Compacting",
                            Some(progress(done_bytes, total_bytes)),
                        );
                    }
                    Ok(GuiRequest::Stop) if !stopped => {
                        stopped = true;
                        self.gui.status(
                            if pending.is_empty() {
                                "Stopping"
                            } else {
                                "Stopping after active files finish"
                            },
                            Some(progress(done_bytes, total_bytes)),
                        );
                    }
                    Ok(_) => (),
                    Err(_) => break,
                }
            }

            match recv_result.recv_timeout(Duration::from_millis(25)) {
                Ok((path, result)) => {
                    let Some(mut fi) = pending.remove(&path) else {
                        continue;
                    };
                    let logical_size = fi.logical_size;
                    let display_path = fi.path.clone();
                    done_files += 1;
                    done_bytes = done_bytes.saturating_add(logical_size);

                    match result {
                        Ok(true) => {
                            fi.physical_size = path.size_on_disk().unwrap_or(fi.physical_size);
                            fi.estimated_physical_size = fi.physical_size;

                            if fi.physical_size >= fi.logical_size {
                                if let Ok(metadata) = std::fs::metadata(&path) {
                                    incompressible.insert(incompressible_key(&path, &metadata));
                                }
                                folder.push(FileKind::Skipped, fi);
                            } else {
                                folder.push(FileKind::Compressed, fi);
                            }
                        }
                        Ok(false) => {
                            fi.estimated_physical_size = fi.physical_size;
                            if let Ok(metadata) = std::fs::metadata(&path) {
                                incompressible.insert(incompressible_key(&path, &metadata));
                            }
                            folder.push(FileKind::Skipped, fi);
                        }
                        Err(err) => {
                            fi.estimated_physical_size = fi.physical_size;
                            self.gui.status(
                                format!("Error: {}, {}", err, display_path.display()),
                                Some(progress(done_bytes, total_bytes)),
                            );
                            folder.push(FileKind::Skipped, fi);
                        }
                    }

                    if last_update.elapsed() > Duration::from_millis(50) {
                        last_update = Instant::now();
                        self.gui.status(
                            if paused {
                                if pending.is_empty() {
                                    "Paused".to_string()
                                } else {
                                    "Pausing after active files finish".to_string()
                                }
                            } else if stopped {
                                if pending.is_empty() {
                                    "Stopping".to_string()
                                } else {
                                    "Stopping after active files finish".to_string()
                                }
                            } else {
                                format!("Compacting: {}", display_path.display())
                            },
                            Some(progress(done_bytes, total_bytes)),
                        );

                        if pending.is_empty() {
                            self.gui.summary(folder.summary());
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if paused && pending.is_empty() && last_update.elapsed() > Duration::from_millis(50)
                    {
                        last_update = Instant::now();
                        self.gui.status(
                            "Paused",
                            Some(progress(done_bytes, total_bytes)),
                        );
                        self.gui.summary(folder.summary());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    stopped = true;
                    break;
                }
            }
        }

        drop(send_file);
        for task in tasks {
            task.wait();
        }
        let _ = incompressible.save();

        let new_size = folder.physical_size;
        let decimal = config().read().unwrap().current().decimal;
        let saved = old_size.saturating_sub(new_size);

        let msg = if stopped {
            format!(
                "Stopped after {} of {} files, saving {} in {:.2?}",
                done_files,
                total_files,
                format_size(saved, decimal),
                start.elapsed()
            )
        } else {
            format!(
                "Compacted {} in {} files, saving {} in {:.2?}",
                format_size(compressible_size, decimal),
                done_files,
                format_size(saved, decimal),
                start.elapsed()
            )
        };

        self.gui
            .status(msg, Some(progress(done_bytes, total_bytes)));
        self.gui.summary(folder.summary());
        if stopped {
            self.gui.stopped();
        } else {
            self.gui.scanned();
        }

        self.info = Some(folder);
    }

    fn uncompress_loop(&mut self) {
        let (send_file, send_file_rx) = bounded::<(PathBuf, u64)>(1);
        let (recv_result_tx, recv_result) = bounded::<(PathBuf, io::Result<bool>)>(1);

        let compactor = BackgroundCompactor::new(None, 0.0, send_file_rx, recv_result_tx);
        let task = BackgroundHandle::spawn(compactor);
        let start = Instant::now();

        let mut folder = self.info.take().expect("fileinfo");
        let summary = folder.summary();
        let total_files = folder.len(FileKind::Compressed);
        let total_bytes = summary.compressed.logical_size;
        let mut done_files = 0usize;
        let mut done_bytes = 0u64;

        let mut last_update = Instant::now();
        let mut paused = false;
        let mut stopped = false;

        let old_size = folder.physical_size;

        self.gui.compacting();
        self.gui.status("Expanding".to_string(), Some(0.0));

        loop {
            while paused && !stopped {
                self.gui.status(
                    "Paused".to_string(),
                    Some(progress(done_bytes, total_bytes)),
                );
                self.gui.summary(folder.summary());

                match self.msg.recv() {
                    Ok(GuiRequest::Pause) => paused = true,
                    Ok(GuiRequest::Resume) => {
                        self.gui.status(
                            "Expanding".to_string(),
                            Some(progress(done_bytes, total_bytes)),
                        );
                        self.gui.resumed();
                        paused = false;
                        last_update = Instant::now();
                    }
                    Ok(GuiRequest::Stop) => {
                        stopped = true;
                        break;
                    }
                    Ok(_) => (),
                    Err(_) => {
                        stopped = true;
                        break;
                    }
                }
            }

            if stopped {
                break;
            }

            if last_update.elapsed() > Duration::from_millis(50) {
                self.gui.status(
                    "Expanding".to_string(),
                    Some(progress(done_bytes, total_bytes)),
                );
                last_update = Instant::now();
                self.gui.summary(folder.summary());
            }

            if let Some(mut fi) = folder.pop(FileKind::Compressed) {
                let logical_size = fi.logical_size;
                send_file
                    .send((folder.path.join(&fi.path), logical_size))
                    .expect("send_file");

                let mut waiting = false;
                loop {
                    if let Ok((_path, result)) = recv_result.recv_timeout(Duration::from_millis(25)) {
                        done_files += 1;
                        done_bytes = done_bytes.saturating_add(logical_size);

                        match result {
                            Ok(_) => {
                                fi.physical_size = fi.logical_size;
                                fi.estimated_physical_size = fi.logical_size;
                                folder.push(FileKind::Compressible, fi);
                            }
                            Err(err) => {
                                fi.estimated_physical_size = fi.physical_size;
                                self.gui.status(
                                    format!("Error: {}, {}", err, fi.path.display()),
                                    Some(progress(done_bytes, total_bytes)),
                                );
                                folder.push(FileKind::Skipped, fi);
                            }
                        }

                        break;
                    }

                    if !waiting && last_update.elapsed() > Duration::from_millis(50) {
                        self.gui.status(
                            format!("Expanding: {}", fi.path.display()),
                            Some(progress(done_bytes, total_bytes)),
                        );
                        last_update = Instant::now();
                        waiting = true;
                    }

                    match self.msg.try_recv() {
                        Ok(GuiRequest::Pause) if !paused => {
                            self.gui.status(
                                format!("Pausing after {}", fi.path.display()),
                                Some(progress(done_bytes, total_bytes)),
                            );
                            self.gui.paused();
                            paused = true;
                        }
                        Ok(GuiRequest::Resume) => {
                            self.gui.resumed();
                            paused = false;
                            stopped = false;
                        }
                        Ok(GuiRequest::Stop) if !stopped => {
                            self.gui.status(
                                format!("Stopping after {}", fi.path.display()),
                                Some(progress(done_bytes, total_bytes)),
                            );
                            stopped = true;
                        }
                        Ok(_) => (),
                        Err(_) => (),
                    }
                }
            } else {
                break;
            }
        }

        drop(send_file);
        task.wait();

        let new_size = folder.physical_size;
        let decimal = config().read().unwrap().current().decimal;
        let wasted = new_size.saturating_sub(old_size);

        let msg = if stopped {
            format!(
                "Stopped after expanding {} of {} files in {:.2?}",
                done_files,
                total_files,
                start.elapsed()
            )
        } else {
            format!(
                "Expanded {} files using {} more space in {:.2?}",
                done_files,
                format_size(wasted, decimal),
                start.elapsed()
            )
        };

        self.gui
            .status(msg, Some(progress(done_bytes, total_bytes)));
        self.gui.summary(folder.summary());
        if stopped {
            self.gui.stopped();
        } else {
            self.gui.scanned();
        }

        self.info = Some(folder);
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_report_lists_folders_and_files() {
        let mut folder = FolderInfo::new(r"C:\Games");
        folder.push(
            FileKind::Compressed,
            FileInfo {
                path: PathBuf::from(r"Data\one.bin"),
                logical_size: 8192,
                physical_size: 4096,
                estimated_physical_size: 4096,
            },
        );
        folder.push(
            FileKind::Compressed,
            FileInfo {
                path: PathBuf::from(r"Data\Sub\two.bin"),
                logical_size: 16384,
                physical_size: 8192,
                estimated_physical_size: 8192,
            },
        );

        let report = build_compressed_report(&folder, false);
        assert!(report.contains("2 compressed files"));
        assert!(report.contains(r"Data\one.bin"));
        assert!(report.contains(r"Data\Sub\two.bin"));
        assert!(report.contains("Folders containing compressed files"));
        assert!(report.contains("Compressed files"));
    }
}
