import os
import posixpath
import re

INDEXES = [
    'docs/plans/plan-sets/set-S/MASTER.md',
    'docs/plans/README.md',
    'docs/adr/README.md',
    'docs/FOLDER_MAP.md',
]

bad = []
checked = 0
for f in INDEXES:
    if not os.path.exists(f):
        print('MISSING INDEX', f)
        continue
    base = os.path.dirname(f).replace(os.sep, '/')
    s = open(f, encoding='utf-8', errors='replace').read()
    for m in re.finditer(r'\[[^\]]*\]\(([^)#]+?)(?:#[^)]*)?\)', s):
        t = m.group(1).strip().replace('%20', ' ')
        if t.startswith(('http://', 'https://', 'mailto:')):
            continue
        p = posixpath.normpath(posixpath.join(base, t))
        checked += 1
        if not os.path.exists(p):
            bad.append((f, t))

print(f'checked {checked} links, {len(bad)} broken')
for f, t in bad:
    print(f'  BROKEN  {f}  ->  {t}')
