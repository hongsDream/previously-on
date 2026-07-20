# PreviouslyOn pilot checklist

Use this one-page checklist beside `previously diagnostics --repo <path>`. Diagnostics stays local,
prints only aggregate schemaVersion 1 JSON, and does not upload or store a report. Review its output
before sharing it.

For each real continuation, record the following manually. Do not paste private prompts, repository
paths, file names, commands, task IDs, or source code into the checklist.

## Before continuing

- Date or pilot round label (not an exact timestamp):
- Was the “Continue in a new task?” suggestion shown at an appropriate time? `yes / early / late / no`
- Did the new task open automatically after consent? `yes / no`

## In the new task

- Could the new task state the current goal without another explanation? `yes / partly / no`
- Number of stale facts noticed:
- Number of important facts missing:
- Number of required tests marked missing, stale, or failed:
- Did any test appear as passed without matching existing evidence? `yes / no`
- Number of manual corrections or context pastes needed before useful work resumed:

## Outcome

- Did the continued task preserve the relevant Regression Contract? `yes / no / not applicable`
- Did it continue in the intended repository and worktree? `yes / no`
- Was any private content present in the diagnostics JSON? `yes / no`
- Short usability note, without source text or identifiers:

Keep the provisional rollover policy (`7` compactions or `80%` context use) separate from this
checklist. Pilot observations do not establish that the threshold is benchmark-validated.
