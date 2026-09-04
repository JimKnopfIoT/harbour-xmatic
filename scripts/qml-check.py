#!/usr/bin/env python3
"""Two ways a QML file refuses to load, neither of which the build reports.

qmllint only parses; "Property value set multiple times" and "id is not
unique" are both raised when the type is *loaded*, which on this project means
at runtime on the device — the page simply fails to push. This walks brace
scopes instead, ignoring braces that live in strings or comments, and reports
a name bound twice in the same scope or an id used twice in one file.
"""
import re
import sys
import pathlib

BINDING = re.compile(r'^\s*((?:on[A-Z]\w*)|[a-z]\w*(?:\.\w+)*)\s*:(?!:)')
# An id is unique per file, not per scope - a component of ours reusing a name
# the page already has takes the whole page down with it.
IDENT = re.compile(r'^\s*id\s*:\s*([A-Za-z_]\w*)\s*$')


def scan(path):
    text = path.read_text()
    scopes = [{}]           # stack of {name: line}
    identifiers = {}        # id -> line, for the whole file
    findings = []
    in_block_comment = False
    for lineno, raw in enumerate(text.splitlines(), 1):
        line = raw
        # strip comments and string literals so their braces do not count
        if in_block_comment:
            end = line.find('*/')
            if end < 0:
                continue
            line = line[end + 2:]
            in_block_comment = False
        line = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line)
        line = re.sub(r"'(?:[^'\\]|\\.)*'", "''", line)
        start = line.find('/*')
        if start >= 0:
            in_block_comment = '*/' not in line[start:]
            line = line[:start] + (line[start:].split('*/', 1)[1]
                                   if not in_block_comment else '')
        line = re.sub(r'//.*$', '', line)

        identifier = IDENT.match(line)
        if identifier:
            name = identifier.group(1)
            previous = identifiers.get(name)
            if previous is not None:
                findings.append((lineno, "id " + name, previous))
            else:
                identifiers[name] = lineno

        match = BINDING.match(line)
        if match:
            name = match.group(1)
            previous = scopes[-1].get(name)
            if previous is not None:
                findings.append((lineno, name, previous))
            else:
                scopes[-1][name] = lineno

        for char in line:
            if char == '{':
                scopes.append({})
            elif char == '}' and len(scopes) > 1:
                scopes.pop()
    return findings


status = 0
for arg in sys.argv[1:]:
    for path in sorted(pathlib.Path(arg).rglob('*.qml')) if pathlib.Path(arg).is_dir() else [pathlib.Path(arg)]:
        for lineno, name, previous in scan(path):
            what = "already used" if name.startswith("id ") else "already bound"
            print(f"{path}:{lineno}: '{name}' {what} at line {previous}")
            status = 1
print("no duplicate bindings or ids" if status == 0 else "", end="")
sys.exit(status)
