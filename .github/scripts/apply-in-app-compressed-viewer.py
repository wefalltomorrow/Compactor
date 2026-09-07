from pathlib import Path
import re

def read(path):
    return Path(path).read_text(encoding='utf-8')

def write(path, text):
    Path(path).write_text(text, encoding='utf-8')

def replace_once(text, old, new, path):
    if old not in text:
        raise RuntimeError(f'Expected text not found in {path}: {old[:120]!r}')
    return text.replace(old, new, 1)

def regex_once(text, pattern, replacement, path):
    new, count = re.subn(pattern, replacement, text, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f'Expected one regex match in {path}, got {count}: {pattern!r}')
    return new

path = 'src/gui.rs'
text = read(path)
text = replace_once(
    text,
    '    Decompress,\n    ViewCompressed,\n    Pause,',
    '''    Decompress,
    ViewCompressed {
        view: String,
        query: String,
        page: usize,
    },
    Pause,''',
    path,
)
text = replace_once(
    text,
    '#[derive(Serialize)]\n#[serde(tag = "type")]\npub enum GuiResponse {',
    '''#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum CompressedViewItem {
    File {
        path: PathBuf,
        logical_size: u64,
        physical_size: u64,
    },
    Folder {
        path: PathBuf,
        count: usize,
        logical_size: u64,
        physical_size: u64,
    },
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum GuiResponse {''',
    path,
)
text = replace_once(
    text,
    '    FolderSummary {\n        info: FolderSummary,\n    },\n    Paused,',
    '''    FolderSummary {
        info: FolderSummary,
    },
    CompressedView {
        root: PathBuf,
        view: String,
        query: String,
        page: usize,
        pages: usize,
        total: usize,
        compressed_count: usize,
        logical_size: u64,
        physical_size: u64,
        items: Vec<CompressedViewItem>,
    },
    Paused,''',
    path,
)
write(path, text)

path = 'src/backend.rs'
text = read(path)
text = replace_once(
    text,
    'use std::collections::{BTreeMap, HashMap};\nuse std::fmt::Write;\nuse std::fs;\nuse std::io;\nuse std::path::{Path, PathBuf};',
    'use std::collections::{BTreeMap, HashMap};\nuse std::io;\nuse std::path::{Path, PathBuf};',
    path,
)
text = replace_once(
    text,
    'use crate::gui::{GuiRequest, GuiWrapper};',
    'use crate::gui::{CompressedViewItem, GuiRequest, GuiResponse, GuiWrapper};',
    path,
)
helpers = r'''const COMPRESSED_VIEW_PAGE_SIZE: usize = 100;

#[derive(Default)]
struct CompressedFolderTotals {
    count: usize,
    logical_size: u64,
    physical_size: u64,
}

fn path_matches_query(path: &Path, query: &str) -> bool {
    query.is_empty() || path.to_string_lossy().to_lowercase().contains(query)
}

fn compressed_file_items(folder: &FolderInfo, query: &str) -> Vec<CompressedViewItem> {
    let mut files: Vec<&FileInfo> = folder
        .compressed
        .files
        .iter()
        .filter(|fi| path_matches_query(&fi.path, query))
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));

    files
        .into_iter()
        .map(|fi| CompressedViewItem::File {
            path: fi.path.clone(),
            logical_size: fi.logical_size,
            physical_size: fi.physical_size,
        })
        .collect()
}

fn compressed_folder_items(folder: &FolderInfo, query: &str) -> Vec<CompressedViewItem> {
    let mut folders: BTreeMap<PathBuf, CompressedFolderTotals> = BTreeMap::new();

    for fi in &folder.compressed.files {
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

    folders
        .into_iter()
        .filter(|(path, _)| path_matches_query(path, query))
        .map(|(path, totals)| CompressedViewItem::Folder {
            path,
            count: totals.count,
            logical_size: totals.logical_size,
            physical_size: totals.physical_size,
        })
        .collect()
}

fn paginate_compressed_items(
    items: Vec<CompressedViewItem>,
    page: usize,
) -> (usize, usize, usize, Vec<CompressedViewItem>) {
    let total = items.len();
    let pages = if total == 0 {
        1
    } else {
        (total + COMPRESSED_VIEW_PAGE_SIZE - 1) / COMPRESSED_VIEW_PAGE_SIZE
    };
    let page = page.min(pages - 1);
    let start = page.saturating_mul(COMPRESSED_VIEW_PAGE_SIZE);
    let page_items = items
        .into_iter()
        .skip(start)
        .take(COMPRESSED_VIEW_PAGE_SIZE)
        .collect();

    (page, pages, total, page_items)
}

'''
text = regex_once(
    text,
    r'\n#\[derive\(Default\)\]\nstruct CompressedFolderTotals \{.*?\n\}\n\nimpl<T> Backend<T> \{',
    '\n' + helpers + 'impl<T> Backend<T> {',
    path,
)
text = replace_once(
    text,
    '                Ok(GuiRequest::ViewCompressed) if self.info.is_some() => {\n                    self.view_compressed();\n                }',
    '                Ok(GuiRequest::ViewCompressed { view, query, page }) if self.info.is_some() => {\n                    self.view_compressed(view, query, page);\n                }',
    path,
)
new_view = r'''    fn view_compressed(&self, view: String, query: String, page: usize) {
        let Some(folder) = self.info.as_ref() else {
            return;
        };

        let query = query.trim().to_string();
        let query_match = query.to_lowercase();
        let view = if view.eq_ignore_ascii_case("folders") {
            "folders"
        } else {
            "files"
        };
        let items = if view == "folders" {
            compressed_folder_items(folder, &query_match)
        } else {
            compressed_file_items(folder, &query_match)
        };
        let (page, pages, total, items) = paginate_compressed_items(items, page);
        let summary = folder.compressed.summary();

        self.gui.send(&GuiResponse::CompressedView {
            root: folder.path.clone(),
            view: view.to_string(),
            query,
            page,
            pages,
            total,
            compressed_count: summary.count,
            logical_size: summary.logical_size,
            physical_size: summary.physical_size,
            items,
        });
    }

'''
text = regex_once(
    text,
    r'    fn view_compressed\(&self\) \{.*?\n    \}\n\n    fn compress_loop',
    new_view + '    fn compress_loop',
    path,
)
tests = r'''#[cfg(test)]
mod tests {
    use super::*;

    fn sample_compressed_folder() -> FolderInfo {
        let mut folder = FolderInfo::new(PathBuf::from("C:").join("Games"));

        for (path, logical_size, physical_size) in [
            (PathBuf::from("Data").join("one.bin"), 8192, 4096),
            (PathBuf::from("Data").join("two.bin"), 16384, 8192),
            (
                PathBuf::from("Data").join("Sub").join("three.bin"),
                32768,
                12288,
            ),
        ] {
            folder.push(
                FileKind::Compressed,
                FileInfo {
                    path,
                    logical_size,
                    physical_size,
                    estimated_physical_size: physical_size,
                },
            );
        }

        folder
    }

    #[test]
    fn compressed_file_view_filters_paths() {
        let folder = sample_compressed_folder();
        let items = compressed_file_items(&folder, "sub");

        assert_eq!(1, items.len());
        match &items[0] {
            CompressedViewItem::File { path, .. } => {
                assert_eq!(
                    &PathBuf::from("Data").join("Sub").join("three.bin"),
                    path
                );
            }
            _ => panic!("expected file item"),
        }
    }

    #[test]
    fn compressed_folder_view_groups_files() {
        let folder = sample_compressed_folder();
        let items = compressed_folder_items(&folder, "");

        assert_eq!(2, items.len());
        let data = items
            .iter()
            .find(|item| match item {
                CompressedViewItem::Folder { path, .. } => path == &PathBuf::from("Data"),
                _ => false,
            })
            .expect("Data folder");

        match data {
            CompressedViewItem::Folder {
                count,
                logical_size,
                physical_size,
                ..
            } => {
                assert_eq!(2, *count);
                assert_eq!(24576, *logical_size);
                assert_eq!(12288, *physical_size);
            }
            _ => panic!("expected folder item"),
        }
    }

    #[test]
    fn compressed_view_pagination_clamps_page() {
        let items = (0..3)
            .map(|index| CompressedViewItem::File {
                path: PathBuf::from(format!("{}.bin", index)),
                logical_size: 8192,
                physical_size: 4096,
            })
            .collect();

        let (page, pages, total, page_items) = paginate_compressed_items(items, usize::MAX);
        assert_eq!(0, page);
        assert_eq!(1, pages);
        assert_eq!(3, total);
        assert_eq!(3, page_items.len());
    }
}
'''
text = regex_once(
    text,
    r'\n\n#\[cfg\(test\)\]\nmod tests \{.*\}\s*$',
    '\n\n' + tests,
    path,
)
write(path, text)

path = 'src/ui/app.js'
text = read(path)
text = replace_once(
    text,
    "\t\tview_compressed: function() {\n\t\t\texternal.invoke(JSON.stringify({ type: 'ViewCompressed' }));\n\t\t},",
    r'''\t\tview_compressed: function(view, query, page) {
\t\t\texternal.invoke(JSON.stringify({
\t\t\t\ttype: 'ViewCompressed',
\t\t\t\tview: view,
\t\t\t\tquery: query,
\t\t\t\tpage: page
\t\t\t}));
\t\t},''',
    path,
)
text = replace_once(
    text,
    '\t\t\t\tcase "FolderSummary":\n\t\t\t\t\tGui.set_folder_summary(msg.info);\n\t\t\t\t\tbreak;',
    '''\t\t\t\tcase "FolderSummary":
\t\t\t\t\tGui.set_folder_summary(msg.info);
\t\t\t\t\tbreak;

\t\t\t\tcase "CompressedView":
\t\t\t\t\tGui.set_compressed_view(msg);
\t\t\t\t\tbreak;''',
    path,
)
gui_helpers = r'''var Gui = (function() {
\t"use strict";

\tvar compressedView = {
\t\tview: "files",
\t\tpage: 0,
\t\tpages: 1
\t};

\tvar compressedSearch = Util.debounce(function() {
\t\tGui.request_compressed(0);
\t}, 200);

\tvar addTableCell = function(row, value, className) {
\t\tvar cell = $("<td></td>").text(value);
\t\tif (className) cell.addClass(className);
\t\trow.append(cell);
\t};

\treturn {'''
text = replace_once(
    text,
    'var Gui = (function() {\n\t"use strict";\n\n\treturn {',
    gui_helpers,
    path,
)
text = replace_once(
    text,
    '\t\t\t$("#Button_Reset").on("click", function() {\n\t\t\t\tAction.reset_config();\n\t\t\t});\n\t\t},',
    '''\t\t\t$("#Button_Reset").on("click", function() {
\t\t\t\tAction.reset_config();
\t\t\t});

\t\t\t$("#Compressed_View_Search").on("input", compressedSearch);
\t\t},''',
    path,
)
viewer_methods = r'''
\t\topen_compressed_view: function() {
\t\t\tcompressedView.view = "files";
\t\t\tcompressedView.page = 0;
\t\t\tcompressedView.pages = 1;
\t\t\t$("#Compressed_View_Search").val("");
\t\t\tGui.page("CompressedFiles");
\t\t\tGui.request_compressed(0);
\t\t},

\t\trequest_compressed: function(page) {
\t\t\tvar query = $("#Compressed_View_Search").val() || "";
\t\t\tvar requestedPage = parseInt(page, 10);
\t\t\tif (isNaN(requestedPage) || requestedPage < 0) requestedPage = 0;
\t\t\tAction.view_compressed(compressedView.view, query, requestedPage);
\t\t},

\t\tset_compressed_mode: function(view) {
\t\t\tcompressedView.view = view == "folders" ? "folders" : "files";
\t\t\tGui.request_compressed(0);
\t\t},

\t\tcompressed_page: function(delta) {
\t\t\tGui.request_compressed(compressedView.page + delta);
\t\t},

\t\tset_compressed_view: function(data) {
\t\t\tcompressedView.view = data.view == "folders" ? "folders" : "files";
\t\t\tcompressedView.page = data.page;
\t\t\tcompressedView.pages = data.pages;

\t\t\t$("#Compressed_View_Root").text(data.root);
\t\t\t$("#Compressed_View_Summary").text(
\t\t\t\tUtil.format_number(data.compressed_count, 0) + " compressed files - " +
\t\t\t\tUtil.bytes_to_human(data.logical_size) + " logical - " +
\t\t\t\tUtil.bytes_to_human(data.physical_size) + " on-disk - " +
\t\t\t\tUtil.bytes_to_human(Math.max(0, data.logical_size - data.physical_size)) + " saved"
\t\t\t);

\t\t\t$(".compressed-view-mode").removeClass("active");
\t\t\tif (compressedView.view == "folders") {
\t\t\t\t$("#Button_Compressed_Folders").addClass("active");
\t\t\t} else {
\t\t\t\t$("#Button_Compressed_Files").addClass("active");
\t\t\t}

\t\t\tvar head = $("#Compressed_View_Head").empty();
\t\t\tvar body = $("#Compressed_View_Body").empty();
\t\t\tvar header = $("<tr></tr>");

\t\t\tif (compressedView.view == "folders") {
\t\t\t\taddTableCell(header, "Files");
\t\t\t}
\t\t\taddTableCell(header, "Logical");
\t\t\taddTableCell(header, "On-disk");
\t\t\taddTableCell(header, "Saved");
\t\t\taddTableCell(header, compressedView.view == "folders" ? "Folder" : "File", "path");
\t\t\thead.append(header);

\t\t\tdata.items.forEach(function(item) {
\t\t\t\tvar row = $("<tr></tr>");
\t\t\t\tif (compressedView.view == "folders") {
\t\t\t\t\taddTableCell(row, Util.format_number(item.count, 0));
\t\t\t\t}
\t\t\t\taddTableCell(row, Util.bytes_to_human(item.logical_size));
\t\t\t\taddTableCell(row, Util.bytes_to_human(item.physical_size));
\t\t\t\taddTableCell(row, Util.bytes_to_human(Math.max(0, item.logical_size - item.physical_size)));
\t\t\t\taddTableCell(row, item.path, "path");
\t\t\t\tbody.append(row);
\t\t\t});

\t\t\tif (data.items.length === 0) {
\t\t\t\tvar empty = $("<tr></tr>");
\t\t\t\tempty.append(
\t\t\t\t\t$("<td></td>")
\t\t\t\t\t\t.attr("colspan", compressedView.view == "folders" ? 5 : 4)
\t\t\t\t\t\t.addClass("empty")
\t\t\t\t\t\t.text("No matching compressed " + compressedView.view)
\t\t\t\t);
\t\t\t\tbody.append(empty);
\t\t\t}

\t\t\t$("#Compressed_View_Page").text(
\t\t\t\t"Page " + Util.format_number(data.page + 1, 0) + " of " +
\t\t\t\tUtil.format_number(data.pages, 0) + " - " +
\t\t\t\tUtil.format_number(data.total, 0) + " " + compressedView.view
\t\t\t);
\t\t\t$("#Button_Compressed_Previous").prop("disabled", data.page <= 0);
\t\t\t$("#Button_Compressed_Next").prop("disabled", data.page + 1 >= data.pages);
\t\t},
'''
text = replace_once(
    text,
    '\t\tpage: function(page) {\n\t\t\t$("nav button").removeClass("active");\n\t\t\t$("#Button_Page_" + page).addClass("active");\n\t\t\t$("section.page").hide();\n\t\t\t$("#" + page).show();\n\t\t},\n\n\t\tversion:',
    '\t\tpage: function(page) {\n\t\t\t$("nav button").removeClass("active");\n\t\t\t$("#Button_Page_" + page).addClass("active");\n\t\t\t$("section.page").hide();\n\t\t\t$("#" + page).show();\n\t\t},\n' + viewer_methods + '\n\t\tversion:',
    path,
)
write(path, text)

path = 'src/ui/index.html'
text = read(path)
text = replace_once(
    text,
    'title="Open a report of compressed files and their containing folders" onclick="Action.view_compressed()">View</button>',
    'title="View compressed files and their containing folders" onclick="Gui.open_compressed_view()">View</button>',
    path,
)
viewer_html = r'''    <section class="page" id="CompressedFiles" style="display: none;">
      <div id="Compressed_View_Header">
        <button class="analyse" onclick="Gui.page('Compress')">Back</button>
        <strong>Compressed Files</strong>
      </div>

      <div id="Compressed_View_Root"></div>
      <div id="Compressed_View_Summary"></div>

      <div id="Compressed_View_Controls">
        <button id="Button_Compressed_Files" class="compressed-view-mode active" onclick="Gui.set_compressed_mode('files')">Files</button>
        <button id="Button_Compressed_Folders" class="compressed-view-mode" onclick="Gui.set_compressed_mode('folders')">Folders</button>
        <input id="Compressed_View_Search" type="search" placeholder="Filter by path" autocomplete="off" spellcheck="false">
      </div>

      <div id="Compressed_View_Table_Wrap">
        <table id="Compressed_View_Table">
          <thead id="Compressed_View_Head"></thead>
          <tbody id="Compressed_View_Body"></tbody>
        </table>
      </div>

      <div id="Compressed_View_Pagination" class="ctr">
        <button id="Button_Compressed_Previous" class="analyse" onclick="Gui.compressed_page(-1)">Previous</button>
        <span id="Compressed_View_Page"></span>
        <button id="Button_Compressed_Next" class="analyse" onclick="Gui.compressed_page(1)">Next</button>
      </div>
    </section>

'''
text = replace_once(
    text,
    '    <section class="page" id="Settings" style="display: none;">',
    viewer_html + '    <section class="page" id="Settings" style="display: none;">',
    path,
)
write(path, text)

path = 'src/ui/style.css'
text = read(path)
viewer_css = r'''

#CompressedFiles {
  margin: 12px 16px;
}

#Compressed_View_Header {
  display: flex;
  align-items: center;
  margin-bottom: 4px;
}

#Compressed_View_Header button {
  background-color: rgb(118, 118, 118);
  margin-left: 0;
  margin-right: 8px;
}

#Compressed_View_Root {
  color: #bbb;
  font-size: 0.85em;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

#Compressed_View_Summary {
  margin: 4px 0 8px 0;
  font-size: 0.9em;
}

#Compressed_View_Controls {
  display: flex;
  align-items: center;
  margin-bottom: 6px;
}

#Compressed_View_Controls button {
  background-color: rgb(118, 118, 118);
}

#Compressed_View_Controls button.active {
  background-color: #229af7;
}

#Compressed_View_Search {
  flex: 1;
  min-width: 120px;
  margin-left: 8px;
  font-size: 1em;
}

#Compressed_View_Table_Wrap {
  max-height: 230px;
  overflow: auto;
  border: 1px solid #777;
  background-color: #333;
}

#Compressed_View_Table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.82em;
}

#Compressed_View_Table th,
#Compressed_View_Table td {
  border-bottom: 1px solid #555;
  padding: 4px 6px;
  text-align: right;
  white-space: nowrap;
}

#Compressed_View_Table th {
  background-color: #2c2c2c;
  color: #ddd;
}

#Compressed_View_Table th.path,
#Compressed_View_Table td.path {
  text-align: left;
}

#Compressed_View_Table td.empty {
  padding: 16px;
  text-align: center;
  color: #aaa;
}

#Compressed_View_Pagination {
  align-items: center;
  margin-top: 6px;
}

#Compressed_View_Page {
  min-width: 220px;
  text-align: center;
  font-size: 0.85em;
}

#Compressed_View_Pagination button:disabled {
  cursor: default;
  opacity: 0.45;
}
'''
text = text.rstrip() + viewer_css + '\n'
write(path, text)

path = 'README.md'
text = read(path)
text = text.replace(
    '- On-demand report of WOF-compressed files and their containing folders',
    '- In-app searchable viewer for WOF-compressed files and their containing folders',
)
text = text.replace(
    'Use Decompress to remove WOF backing from files previously compressed with Compactor. After analysis, select View beside the compressed count to open a searchable text report of compressed files and the folders containing them.',
    'Use Decompress to remove WOF backing from files previously compressed with Compactor. After analysis, select View beside the compressed count to browse compressed files and folders inside Compactor. The viewer includes path filtering and pagination for large scans.',
)
text = text.replace(
    '- On-demand compressed-file reporting',
    '- In-app compressed-file and folder viewer with filtering and pagination',
)
write(path, text)

path = 'CHANGELOG.md'
text = read(path)
text = replace_once(
    text,
    '- On-demand report listing WOF-compressed files and the folders containing them.',
    '- In-app viewer for WOF-compressed files and containing folders, with path filtering and pagination for large scans.',
    path,
)
write(path, text)
