#!/usr/bin/env python3
"""Turn a .ts file into a review sheet a non-developer can annotate.

    tools/translation-review.py nb > review-nb.md
    tools/translation-review.py nb todo.txt > review-nb.md   # only these sources

Grouped by the context name, which is the QML page the string lives on, so a
reviewer can find it in the running app. Numbered so corrections can come back
as "217: ...".
"""
import html
import re
import sys

# Per language, what the machine pass was least sure about. Shown at the top so
# a reviewer spends their attention there first.
OPEN_QUESTIONS = {
    "nb": [
        "`tjener` or `server`? Used throughout for *server*. Språkrådet "
        "recommends `tjener`, most Norwegian software writes `server`.",
        "`dekkevne` for *opacity* (the bubble slider). It said "
        "`gjennomsiktighet` first, which is the opposite.",
        "`space` is left in English, as in German. Correct for Norwegian, or "
        "does it need a native word?",
        "`Bokmål` only. A phone set to `nn_NO` currently shows English.",
    ],
}


def load(path):
    text = open(path, encoding="utf-8").read()
    language = re.search(r'<TS[^>]*language="([^"]+)"', text)
    out = []
    for context in re.finditer(r"<context>(.*?)</context>", text, re.S):
        body = context.group(1)
        name = re.search(r"<name>(.*?)</name>", body, re.S).group(1)
        entries = []
        for message in re.finditer(r"<message.*?</message>", body, re.S):
            block = message.group(0)
            source = html.unescape(
                re.search(r"<source>(.*?)</source>", block, re.S).group(1))
            if 'numerus="yes"' in block:
                forms = [html.unescape(f) for f in
                         re.findall(r"<numerusform>(.*?)</numerusform>", block, re.S)]
                target = "  /  ".join(forms)
            else:
                target = html.unescape(
                    re.search(r"<translation>(.*?)</translation>", block, re.S).group(1))
            entries.append((source, target))
        if entries:
            out.append((name, entries))
    return language.group(1) if language else "?", out


def main(code, only=None):
    language, contexts = load("translations/harbour-xmatic-%s.ts" % code)
    if only:
        wanted = set(open(only, encoding="utf-8").read().split("\n"))
        contexts = [(name, [e for e in entries if e[0] in wanted])
                    for name, entries in contexts]
        contexts = [c for c in contexts if c[1]]
    print("# xmatic — translation review: %s\n" % language)
    print("Left is the English original, right is what the app currently shows.")
    print("Reply with the number and a better wording; anything you do not")
    print("mention stays as it is. Only the German translation has had a native")
    print("speaker so far.\n")
    print("`%1`, `%2` and `%n` are filled in by the app — a name, a count. They")
    print("have to survive, but may move within the sentence. `%n` entries show")
    print("one form per line, singular first.\n")
    print("Best read with the app open: **Account → Language** switches between")
    print("English and this language without reinstalling (it takes effect at the")
    print("next start), so a line can be compared where it actually appears. The")
    print("heading above each block is the page the strings live on.\n")
    if code in OPEN_QUESTIONS:
        print("## Where the machine pass was unsure\n")
        for question in OPEN_QUESTIONS[code]:
            print("- " + question)
        print()
    number = 0
    for name, entries in contexts:
        print("## %s\n" % name)
        for source, target in entries:
            number += 1
            print("%d. `%s`" % (number, source.replace("\n", " ")))
            print("   → **%s**\n" % target.replace("\n", " "))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None)
