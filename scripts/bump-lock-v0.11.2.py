from pathlib import Path

p = Path("Cargo.lock")
text = p.read_text(encoding="utf-8")
old = 'name = "compactor"\nversion = "0.11.1"'
new = 'name = "compactor"\nversion = "0.11.2"'
if text.count(old) != 1:
    raise SystemExit("Expected exactly one Compactor 0.11.1 package entry in Cargo.lock")
p.write_text(text.replace(old, new, 1), encoding="utf-8", newline="\n")
