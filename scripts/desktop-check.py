#!/usr/bin/env python3
"""The two desktop files must grant the same thing.

The link handler and the app carry their own `Permissions=` line, and the file
says in a comment that they are the same list — which they were not: one entry
was missing for two releases. A comment is not a mechanism, so this is one.

Sailjail drops a permission it does not know without a word, and whichever
file started the process decides what it may do. The one that is short is
therefore a second, weaker profile for the same application, and nothing at
runtime says so.
"""
import pathlib
import re
import sys

MAIN = "harbour-xmatic.desktop"
HANDLER = "harbour-xmatic-open-url.desktop"


def permissions(path):
    match = re.search(r"^Permissions=(.*)$", path.read_text(), re.M)
    if not match:
        return None
    return [entry for entry in match.group(1).split(";") if entry]


root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else ".")
main, handler = permissions(root / MAIN), permissions(root / HANDLER)

if main is None or handler is None:
    missing = MAIN if main is None else HANDLER
    print(f"{missing}: no Permissions= line")
    sys.exit(1)

only_main = [entry for entry in main if entry not in handler]
only_handler = [entry for entry in handler if entry not in main]

if only_main or only_handler:
    for entry in only_main:
        print(f"{HANDLER}: '{entry}' is granted to the app and not to the handler")
    for entry in only_handler:
        print(f"{MAIN}: '{entry}' is granted to the handler and not to the app")
    sys.exit(1)

print("the two desktop files grant the same permissions", end="")
