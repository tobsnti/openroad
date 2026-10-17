# Evidence — 339 unfilled rows, and one of them is a login error message

Branch for PR *"textdata: an unfilled row is not a message"*
(`fix/unfilled-row-is-not-a-message`). **No image here, on purpose** — the
difference is a string. A screenshot would need a server that sends login error
`0x0B`, and `main` has no tool to produce one; the number is the evidence.

## How to reproduce

```
python3 count_unfilled_rows.py <SRO_PATH>/Media/server_dep/silkroad/textdata/textuisystem.txt <openroad checkout>
```

The script's parser is a **replica** of `UiSystemText::parse`
(`client/src/assets/textdata/uisystem.rs`): UTF-16LE, tab separated, column 0 must
be `1`, column 1 is the key, the value is the **last non-empty** column. Its row
count is cross-checked against the client itself, which logs `loaded 5324 UI
strings` for the same file.

## Output on the data used here (`output.txt`)

```
rows parsed:              5324
rows that are digits only:   350
rows that are exactly 0:     339

DIFFERS    UIIO_CLIENT_START_CONTENT_FAIL_BILLING_FAILED
  data:     '0'
  fallback: 'Billing failed. Cannot establish connection.'
DIFFERS    UIO_MSG_ERROR_CHARACTER_NAME_STRING
  data:     'Exceeded the letter limit. \\nOnly 12 English letters are available.[Min., Max.]'
  fallback: 'Exceeded the letter limit. \nOnly 12 English letters are available.[Min., Max.]'

error-table keys: 21  identical: 19  differs: 2  missing: 0
```

Read that in three parts:

1. **339 rows are the bare digit `0`.** That is how this data set marks an entry
   nobody filled in — not an empty column.
2. **One of them is a message the player can hit:**
   `UIIO_CLIENT_START_CONTENT_FAIL_BILLING_FAILED` is the text for login error
   `0x0B`. Before this change the lookup preferred the row, so the explanation for
   a failed login was the string `0`.
3. **19 of 21 error-table keys are byte-identical** to the shipped fallbacks, so
   the data path is doing its job everywhere else and keeps priority. The change
   does not retreat from the data, it skips rows that carry no sentence.

## The second divergence is deliberately left alone

`UIO_MSG_ERROR_CHARACTER_NAME_STRING` carries a **literal backslash-n** in the
data where our fallback has a real line break. Whether the original resolves that
escape when it renders the row is **unknown**, so nothing is changed for it here.

## Limits

* Measured against **one** user's `textuisystem.txt`. Another data set may fill the
  row — and then it wins again, which is the point of the lookup order.
* The script reads the user's data; **no data is included on this branch**.
