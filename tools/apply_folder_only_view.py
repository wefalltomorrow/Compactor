from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected 1 match, found {count}")
    return text.replace(old, new, 1)


# UI markup
path = Path("src/ui/index.html")
text = path.read_text(encoding="utf-8")
text = replace_once(
    text,
    'title="View compressed files and their containing folders"',
    'title="View folders containing compressed files"',
    "view button title",
)
text = replace_once(text, "<strong>Compressed Files</strong>", "<strong>Compressed Folders</strong>", "viewer title")
text = replace_once(
    text,
    '''      <div id="Compressed_View_Controls">
        <button id="Button_Compressed_Files" class="compressed-view-mode active" onclick="Gui.set_compressed_mode('files')">Files</button>
        <button id="Button_Compressed_Folders" class="compressed-view-mode" onclick="Gui.set_compressed_mode('folders')">Folders</button>
        <input id="Compressed_View_Search" type="search" placeholder="Filter by path" autocomplete="off" spellcheck="false">
      </div>''',
    '''      <div id="Compressed_View_Controls">
        <input id="Compressed_View_Search" type="search" placeholder="Filter folders by path" autocomplete="off" spellcheck="false">
      </div>''',
    "viewer controls",
)
path.write_text(text, encoding="utf-8")


# Embedded UI behavior
path = Path("src/ui/app.js")
text = path.read_text(encoding="utf-8")
text = replace_once(text, '\t\tview: "files",', '\t\tview: "folders",', "default viewer mode")
text = replace_once(text, '\t\t\tcompressedView.view = "files";', '\t\t\tcompressedView.view = "folders";', "open viewer mode")
text = replace_once(
    text,
    '''\t\tset_compressed_mode: function(view) {
\t\t\tcompressedView.view = view == "folders" ? "folders" : "files";
\t\t\tGui.request_compressed(0);
\t\t},

''',
    "",
    "mode switch function",
)
text = replace_once(
    text,
    '\t\t\tcompressedView.view = data.view == "folders" ? "folders" : "files";',
    '\t\t\tcompressedView.view = "folders";',
    "response viewer mode",
)
text = replace_once(
    text,
    '''\t\t\t$(".compressed-view-mode").removeClass("active");
\t\t\tif (compressedView.view == "folders") {
\t\t\t\t$("#Button_Compressed_Folders").addClass("active");
\t\t\t} else {
\t\t\t\t$("#Button_Compressed_Files").addClass("active");
\t\t\t}

''',
    "",
    "mode button state",
)
text = replace_once(
    text,
    '''\t\t\tif (compressedView.view == "folders") {
\t\t\t\taddTableCell(header, "Files");
\t\t\t}
\t\t\taddTableCell(header, "Logical");
\t\t\taddTableCell(header, "On-disk");
\t\t\taddTableCell(header, "Saved");
\t\t\taddTableCell(header, compressedView.view == "folders" ? "Folder" : "File", "path");''',
    '''\t\t\taddTableCell(header, "Files");
\t\t\taddTableCell(header, "Logical");
\t\t\taddTableCell(header, "On-disk");
\t\t\taddTableCell(header, "Saved");
\t\t\taddTableCell(header, "Folder", "path");''',
    "folder table header",
)
text = replace_once(
    text,
    '''\t\t\tdata.items.forEach(function(item) {
\t\t\t\tvar row = $("<tr></tr>");
\t\t\t\tif (compressedView.view == "folders") {
\t\t\t\t\taddTableCell(row, Util.format_number(item.count, 0));
\t\t\t\t}''',
    '''\t\t\tdata.items.forEach(function(item) {
\t\t\t\tvar row = $("<tr></tr>");
\t\t\t\taddTableCell(row, Util.format_number(item.count, 0));''',
    "folder table rows",
)
text = replace_once(
    text,
    '\t\t\t\t\t\t.attr("colspan", compressedView.view == "folders" ? 5 : 4)',
    '\t\t\t\t\t\t.attr("colspan", 5)',
    "empty row colspan",
)
text = replace_once(
    text,
    '\t\t\t\t\t\t.text("No matching compressed " + compressedView.view)',
    '\t\t\t\t\t\t.text("No matching compressed folders")',
    "empty row text",
)
text = replace_once(
    text,
    '\t\t\t\tUtil.format_number(data.total, 0) + " " + compressedView.view',
    '\t\t\t\tUtil.format_number(data.total, 0) + " folders"',
    "pagination label",
)
path.write_text(text, encoding="utf-8")


# Documentation
path = Path("README.md")
text = path.read_text(encoding="utf-8")
text = replace_once(
    text,
    "- In-app searchable viewer for WOF-compressed files and their containing folders",
    "- In-app searchable viewer for folders containing WOF-compressed files",
    "README feature",
)
text = replace_once(
    text,
    "After analysis, select View beside the compressed count to browse compressed files and folders inside Compactor. The viewer includes path filtering and pagination for large scans.",
    "After analysis, select View beside the compressed count to browse folders containing WOF-compressed files inside Compactor. The viewer includes path filtering and pagination for large scans.",
    "README usage",
)
text = replace_once(
    text,
    "- In-app compressed-file and folder viewer with filtering and pagination",
    "- In-app compressed-folder viewer with filtering and pagination",
    "README upstream changes",
)
path.write_text(text, encoding="utf-8")

path = Path("CHANGELOG.md")
text = path.read_text(encoding="utf-8")
text = replace_once(
    text,
    "- In-app viewer for WOF-compressed files and containing folders, with path filtering and pagination for large scans.",
    "- In-app viewer for folders containing WOF-compressed files, with path filtering and pagination for large scans.",
    "changelog viewer entry",
)
path.write_text(text, encoding="utf-8")
