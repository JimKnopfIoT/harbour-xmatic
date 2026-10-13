#!/usr/bin/env python3
"""JavaScript the Qt 5.6 engine does not have, in QML and JS files.

V4 in Qt 5.6 is ES5: these throw or fail to parse at run time only, and a
throwing statement ends the whole handler it sits in. Exit 1 on any finding.
"""
import pathlib
import re
import sys

PATTERNS = [
    (r'\bQt\.callLater\b', 'Qt.callLater (Qt 5.8)'),
    (r'\bset(?:Timeout|Interval)\s*\(', 'setTimeout/setInterval (no such global)'),
    (r'\bPromise\b|\bnew\s+(?:Map|Set|WeakMap|WeakSet)\b|\bSymbol\s*\(', 'ES6 built-in'),
    (r'\bObject\.(?:assign|values|entries)\b|\bArray\.(?:from|of)\b|\bNumber\.is', 'ES6 static'),
    (r'\.(?:includes|findIndex|startsWith|endsWith|padStart|padEnd|repeat|trimStart|trimEnd|flat|codePointAt|normalize)\s*\(',
     'ES6 method'),
    # Array.prototype.find; Silica's pageStack.find is the one allowed.
    (r'(?<!pageStack)\.find\s*\(', 'Array.find (ES6)'),
    (r'(?:^|[^.\w])class\s+[A-Za-z_$]', 'class'),
    (r'function\s*\w*\s*\([^)]*=', 'default parameter'),
    (r'(?:^|[^.\w])(?:var|let|const)\s*[\[{]', 'destructuring'),
    (r'\{\s*[A-Za-z_$][\w$]*\s*(?:,\s*[A-Za-z_$][\w$]*\s*)*\}\s*(?:[;)]|$)', 'object shorthand'),
    (r'=>', 'arrow function'),
    (r'(?:^|[^.\w])(?:let|const)\s+[A-Za-z_$]', 'let/const'),
    (r'\.\.\.[A-Za-z_$\[]', 'spread'),
    (r'\?\?|\?\.[A-Za-z_$]', 'nullish/optional chaining'),
    (r'\bfor\s*\([^)]*\bof\b', 'for...of'),
    (r'`', 'template literal'),
]
COMPILED = [(re.compile(p), why) for p, why in PATTERNS]
STRING = re.compile(r'"(?:\\.|[^"\\\n])*"|\'(?:\\.|[^\'\\\n])*\'')
# A regex literal where an expression starts; its characters are not code.
REGEX = re.compile(r'(^|[(,=:!&|?{};]|\breturn)\s*/(?![*/])(?:\\.|\[(?:\\.|[^\]\\\n])*\]|[^/\\\n\[])+/[gimuy]*')


def code_of(line):
    line = REGEX.sub(lambda match: match.group(1) + ' ""', line)
    line = STRING.sub('""', line)
    cut = line.find('//')
    return line if cut < 0 else line[:cut]


def scan(path):
    findings = []
    in_comment = False
    for number, raw in enumerate(path.read_text(encoding='utf-8').splitlines(), 1):
        line = raw
        if in_comment:
            end = line.find('*/')
            if end < 0:
                continue
            line = line[end + 2:]
            in_comment = False
        start = line.find('/*')
        if start >= 0 and line.find('*/', start) < 0:
            line = line[:start]
            in_comment = True
        code = code_of(line)
        for pattern, why in COMPILED:
            if pattern.search(code):
                findings.append('%s:%d: %s' % (path, number, why))
    return findings


def main(root):
    findings = []
    for path in sorted(pathlib.Path(root).rglob('*')):
        if path.suffix in ('.qml', '.js'):
            findings += scan(path)
    for finding in findings:
        print(finding)
    return 1 if findings else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else 'qml'))
