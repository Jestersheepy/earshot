# Roadmap

Ideas for using the local models while they're already loaded, so Earshot stays at about 18 MB when idle.
Qwen3-4B is loaded while a meeting is summarised; LFM2-1.2B stays warm for 5 minutes after a dictation.

## Done (next release)

1. **Meeting titles, gist and chapters.** In the same Qwen load as the summary: a title from what the meeting was
   about, a one-sentence gist for lists and the "saved" toast, and topic chapters with times you can jump to.
   Summarising an older meeting again fills these in.
2. **Voice-note titles and triage.** LFM2 names each voice note and sorts it into note, to-do, idea or reminder;
   to-dos and reminders become tasks. Its reply is held to a JSON schema, and a due date is only kept when the
   note says it ("by Monday"); the date itself is worked out in code, not by the model.
3. **Action items as tasks.** Owner, task and due date as a checklist in the note (Obsidian-style `- [ ]`), a Tasks
   view across all notes, and "Add to calendar" (an .ics file) for anything with a date.
4. **Search by meaning, typed or spoken.** A small local embedding model (bge-small, 34 MB) indexes passages of
   every note, ranked together with keyword matches. Typing searches words instantly; Enter searches by meaning.
   "What did Sarah say about pricing last week" is narrowed to Sarah and last week by rules, using only names that
   appear in your notes (LFM2 kept inventing people and dates here). Ask by voice from the mic in the search bar,
   or Ctrl+Alt+F from anywhere. Results open at the matching moment.

## Next

- **Meeting folders.** Group recurring meetings and one-to-ones automatically, by who was in them (the same
  people, or the same named voice), into their own folder.
- **Follow-up email draft** from a meeting's decisions and action items.
- **Speaker name hints** from the transcript ("thanks, Sarah" right after Speaker 2).
- **New dictionary words** offered from names and jargon heard in meetings.
- **Tags and related notes**, linking a meeting to earlier ones on the same topic.
- **Tone per app** for dictation: casual in Slack, polished in Outlook, commit-message style in a terminal.
- **Spoken edits to the last dictation**: "scratch that", "make that a list", "more formal".
- **Smart snippets** that expand from loose wording ("sign it off as usual").
- **An "Answer" button on search**: Qwen reads the top passages and answers with the note and time it came from.

## Later, and opt-in

- **Daily recap and open-items digest.** People mix confidential, project and personal notes, so a recap must be
  off by default and scoped to the folders the user picks, never everything.
- **Keep the summary model loaded**, for people with RAM to spare, so search answers are instant.
