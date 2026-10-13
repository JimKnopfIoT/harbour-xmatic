#!/usr/bin/env python3
"""Every Connections handler aimed at the bridge against the C++ signals.

A handler on a signal that does not exist is a load error at run time only;
check.py cannot see it, because Connections takes any `on...` name. Targets
`matrix` and `matrix.<object>` are resolved through src/*.h. Exit 1 on a find.
Usage: connections.py <repo root>
"""
import pathlib
import re
import sys

BASES = {
    'QAbstractListModel': ['modelReset', 'rowsInserted', 'rowsRemoved', 'dataChanged',
                           'layoutChanged', 'rowsMoved', 'modelAboutToBeReset'],
    'QSortFilterProxyModel': ['modelReset', 'rowsInserted', 'rowsRemoved', 'dataChanged',
                              'layoutChanged', 'rowsMoved'],
    'QObject': ['destroyed', 'objectNameChanged'],
}


def classes(root):
    found = {}
    for header in (root / 'src').glob('*.h'):
        text = header.read_text(encoding='utf-8')
        for match in re.finditer(r'class\s+(\w+)\s*:\s*public\s+(\w+)\s*\{(.*?)\n\};', text, re.S):
            name, base, body = match.groups()
            signals = set()
            for block in re.finditer(r'signals:(.*?)(?=\n\s*(?:public|private|protected)\b|\Z)',
                                     body, re.S):
                for decl in re.finditer(r'void\s+(\w+)\s*\(', block.group(1)):
                    signals.add(decl.group(1))
            found[name] = (base, signals)
    return found


def signals_of(name, known):
    out = set()
    while name:
        if name in known:
            base, own = known[name]
            out |= own
            name = base
        else:
            out |= set(BASES.get(name, []))
            if name in ('QAbstractListModel', 'QSortFilterProxyModel', 'QAbstractItemModel'):
                out |= set(BASES['QAbstractListModel'])
            name = None
    return out


def main(root):
    root = pathlib.Path(root)
    known = classes(root)
    bridge = (root / 'src' / 'matrixbridge.h').read_text(encoding='utf-8')
    members = dict(re.findall(r'QObject\s*\*(\w+)\(\)\s*\{\s*return\s+(m_\w+)', bridge))
    types = {field: kind for kind, field in re.findall(r'(\w+)\s*\*\s*(m_\w+)\s*=', bridge)}

    def class_of(target):
        if target == 'matrix':
            return 'MatrixBridge'
        match = re.match(r'^matrix\.(\w+)$', target)
        if match and match.group(1) in members:
            return types.get(members[match.group(1)])
        return None

    findings = []
    for path in sorted((root / 'qml').rglob('*.qml')):
        lines = path.read_text(encoding='utf-8').split('\n')
        index = 0
        while index < len(lines):
            if not re.match(r'^\s*Connections\s*\{', lines[index]):
                index += 1
                continue
            depth, cursor, target, handlers = 0, index, None, []
            while cursor < len(lines):
                line = lines[cursor]
                if depth == 1:
                    found = re.match(r'^\s*target:\s*(.+?)\s*$', line)
                    if found:
                        target = found.group(1)
                    found = re.match(r'^\s*(on[A-Z]\w*)\s*:', line)
                    if found:
                        handlers.append((found.group(1), cursor + 1))
                depth += line.count('{') - line.count('}')
                cursor += 1
                if depth <= 0:
                    break
            kind = class_of(target or '')
            if kind:
                available = signals_of(kind, known)
                for handler, number in handlers:
                    signal = handler[2].lower() + handler[3:]
                    if signal not in available:
                        findings.append('%s:%d: %s is no signal of %s' % (
                            path.relative_to(root), number, handler, target))
            index = cursor
    for finding in findings:
        print(finding)
    return 1 if findings else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else '.'))
