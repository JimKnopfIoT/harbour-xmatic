.pragma library
// Placeholders filled in one pass. `.arg(a).arg(b)` reads what `a` brought in
// again, so a display name "%2" took the place meant for the second name.

/// `%1` to `%9` in `template` replaced by `values[0]` and on. Nothing a value
/// contains is read as a placeholder; one without a value stays as it is.
function fill(template, values) {
    return String(template).replace(/%([1-9])/g, function(whole, digit) {
        var index = parseInt(digit, 10) - 1
        return index < values.length ? String(values[index]) : whole
    })
}
