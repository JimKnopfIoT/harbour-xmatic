# Translation status

Source language: English. One `.ts` per target; `.qm` is build output. A
language is listed in `TRANSLATIONS` only once it is actually translated —
an empty file would ship as coverage that is not there.

Not shipped, deliberately: `nn` (Norwegian Nynorsk), Sámi, Faroese, and the
non-EU languages east and south of the EU. Adding them means adding more
unreviewed machine output.

`review` is the only column that matters. **machine** means generated in one
pass without a native speaker — usable, not verified.

| lang | language | review | note |
|---|---|---|---|
| en | English | source | identity catalogue, so the picker can override the device |
| de | German | native | project language |
| fi | Finnish | machine, reviewed once | 24 corrections, see below |
| sv | Swedish | machine, reviewed once | 1 correction |
| nb | Norwegian Bokmål | **contributed** | 385 of 491 strings from PR #1; the remaining 106 are newer than it and still machine. `nn_NO` falls back to this. |
| da | Danish | machine, spot-checked | 2 corrections |
| fr es it nl pt | | machine | |
| pl cs sk sl hr bg ru | | machine | plural forms checked against Qt's rule |
| el hu ro et lv lt is | | machine | |
| ga mt | Irish, Maltese | machine, low confidence | 3 and 4 plural forms; smallest corpora |

## Known corrections already applied

- fr `Cœur` → `Noyau` (heart vs. software core)
- ga `Croí` → `Croílár` (same)
- fi `asiakas` → `asiakasohjelma` (customer vs. client)
- nb/is/el/hu/es: GStreamer `pipeline` had become a plumbing pipe, a conveyor
  or "process"; kept as the GStreamer word, which is what a log search uses
- nb/da/fi "opacity" had become "transparency" — the slider's meaning inverted
- fi `tallennus` (the act of saving) → `tallennustila` (storage), `tallentaa`
  (save) → `nauhoittaa` (record), `käytössä` used for both "active" and
  "set up", `kuva` → `profiilikuva`

## Open, needs a native speaker

- nb mixes `tjener` and `server` — the contributed file does too (15 / 16), so
  the inconsistency is not the machine's. Worth one decision.
- nb `Boblegjennomsiktighet` (transparency) for *opacity*, from the contributed
  file. The machine pass had the same word and it was corrected to `dekkevne`
  before the PR turned up; the contributor's wording stands.
- fi inflects `Space` as a loanword (`spaceen`, `spacet`); every other language
  leaves it uninflected.
- `Homeserver` is translated literally in several languages, where it can read
  as "a server in your home".

## Terms deliberately not translated

`Matrix`, `MSC4186`, `simplified sliding sync`, `webrtcbin`, `GStreamer`.
`Space`/`Spaces` stays English in German; Norwegian translates it
(`romgruppe`), on the contributor's decision — a language may go either way.

## Getting a language reviewed

```sh
tools/translation-review.py nb > review-nb.md
```

A numbered sheet grouped by page, with the open questions for that language at
the top. Corrections come back as "217: ..."; apply them, then move the row in
the table above from `machine` to `native`.

## Checking a file

```sh
lrelease-qt5 translations/harbour-xmatic-XX.ts
```

"Removed plural forms as the target language has less forms" means the file
carries more `<numerusform>` entries than Qt allows for that language — one for
Hungarian, three for Irish and most Slavic languages, four for Slovenian and
Maltese, two otherwise.
