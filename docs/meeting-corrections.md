# Meeting corrections

Open a transcribed meeting and expand **Сквозные исправления**. Enter the existing
term in **Как написано**, the preferred spelling in **Как правильно**, and press
**Исправить везде**. An empty replacement removes the term.

Matching uses complete Unicode words, ignoring case. Replacement text is literal,
including dollar signs and regex characters. Rules run in the displayed order.
They apply to the saved transcript, all six current AI report kinds and any
legacy brief. The same rules are applied when this app creates a new transcript
or report for that meeting, including after the optional AI correction pass.
This feature makes no AI request itself.

The per-meeting rule list is stored in the local app WebView profile. Corrected
transcripts and reports are saved to the meeting's actual files. Removing a rule
affects future operations and does not undo saved text. **Применить правила заново**
explicitly reapplies the ordered rules; a replacement containing the original
term can change again on repeated application. Review the result before
reapplying such a rule.

Rule additions/removals merge the current stored list inside the queue, so
reopening a meeting while an action is pending cannot replace newer rules with
an old list. Text writes for the same meeting are queued, including report generation,
transcription, manual saves, corrections and audio edits. Different meetings can
still run independently. The file group is not an atomic transaction: a disk
failure can leave some documents updated. The UI reports this, reloads the saved
state and allows retry; inspect the texts before repeating a replacement.

Manual transcript saves merge fields changed in the user's captured draft into
the current saved transcript. Unrelated completed corrections are retained.
Conflicting edits or changed segment structure are rejected with the draft kept
on screen. Report edits also reject a changed-on-disk base instead of silently
overwriting it. Copy the draft before canceling and reopening a conflicting edit.

This is the current-version port of the useful dictionary proposal in old PR
#36. Its earlier literal-dollar bug and four-report-only list are replaced; old
version/branding changes and local agent instructions are not imported.
