# Sound anchors: fixes that learn how your words sound

2026-11-24, branch `feat/sound-anchors`. A port of the iPhone's design
(`orttakey/Docs/SoundAnchors.md`) to Orttaai for Windows and Linux.

## The problem

You say "emailer" (Eimailar, a product name) and the speech model writes "email". Until now there were two ways to fix that, and both were wrong:

- **A dictionary pair** ("email" → Eimailar). It rewrote every "email" you ever said. Correcting it back made a pair the other way.
- **Word Boost.** It leaves a single ordinary English word alone, so "email" stayed "email" for good. And it could merge "email or" into Eimailar.

The rule now: if you said "email", it stays "email"; if you said your word, it becomes your word. Only the recording can tell the two apart.

## Where it runs

Sound anchors use Word Boost's model (Parakeet CTC 110M), so they are on when all of these hold:

- the engine is Express and dictation is in English;
- **Word Boost** is on (Settings › Model);
- Word Boost's model is installed.

Otherwise dictation works exactly as before. Learning from what you do after a dictation also needs **Learn from my edits** (on by default) and an app whose text Orttaai can read (UI Automation on Windows, AT-SPI on Linux). Nothing in the interface changed.

## What was built

| Piece | File | What it does |
|---|---|---|
| Sound comparison | `core/src/sound/contrast.rs` | Scores "context + heard + context" against "context + your word's sound + context" over the same frames with the CTC forward algorithm. Three words of context and 0.4 s of sound either side; the whole recording when the context reaches both ends. "E-mail" counts as "email". |
| Anchors | `core/src/sound/anchors.rs` | Heard word, your word, up to 4 sounds, the last 12 margins each way, forgotten spellings, and the line a margin must clear. Saved as `sound-anchors.json` in the data folder, in the iPhone's format. Never synced, no audio. |
| Judge | `core/src/sound/judge.rs` | Decides each place the heard word was written (see below). |
| Starting guesses | `core/src/sound/guesses.rs` | A dictionary word spelled like an everyday word plus a lost ending (similarity 0.85 or more) gets an anchor before any fix. Worked out when the dictionary changes and cached, never during a dictation. |
| Slow listen | `core/src/sound/slow_listen.rs` | WSOLA time stretch in plain Rust, pitch kept. |
| Learning | `core/src/sound/learn.rs` | Turns what you do after a dictation into anchors. Keeps the last 3 dictations' sound for 10 minutes, in memory only (recordings over 2 minutes aren't kept). |
| Express | `core/src/transcription/express.rs` | Runs the judge after Word Boost, on the same CTC frames. |
| App | `app/src-tauri/src/sound.rs`, `engine.rs`, `lib.rs`, `pipeline.rs`, `memory.rs` | Loads the anchors with the dictionary, names each watched dictation's sound, learns before any pair is suggested. |

### How one dictation is judged

1. **Spellings of your word's sound are your word.** "emailer", "emailar", "emaila" and "emailor" are not in the English word list, so wherever the speech model writes one, it is Eimailar. So is a near spelling ("Emila", "Emilar": 0.8 alike or more, at least 4 letters, not English). "sounder" is English, so it is left to step 2.
2. **Each "email" is scored against the sound.** It becomes your word when the margin is above the anchor's line. Before an everyday word that can pass for the ending (or, a, at, are, her, of, uh, er) the margin must be 4 above the line, and that bar never drops below 4 however many fixes you make.
3. **Where "email" was kept, the words around it are listened to again**, slowed to 60% and then 70% of the speed. If the speech model then hears your word, one of its spellings, or "email" plus a lost ending (er, ar, a, or, ah, ers, ars), it is your word.

Word Boost no longer decides a word that has an anchor. It also merges an ordinary word and an everyday ending word ("email or") into a term only when the sound is clearly better (margin 2.0 or more).

### What it learns from you

| You do this | It learns |
|---|---|
| Change a dictated ordinary word to a dictionary word ("email" → Eimailar), or to itself plus the ending it lost ("email" → "emailer") | You said your word there. Its sound is learned from the recording. No dictionary pair is suggested. |
| Change a spelling of the sound to your word ("emailer" → Eimailar) | That spelling is your word from now on. |
| Change your word back ("Eimailar" → "email") where the sound wrote it | You didn't say it there. The line moves above that margin. No pair is made the other way. A spelling you put back is never used again. |
| Leave an "email" the sound check kept as it was | One of your own "email"s. With three or more, the line follows your voice (down to −8). |

A fix of a misspelling ("Emila" → Eimailar) still becomes a dictionary suggestion, as before: a pair from a word that isn't English is safe.

An existing pair from an ordinary word to a word with an anchor ("email" → Eimailar) is now decided by the sound. The pair stays in your dictionary, but it no longer rewrites every "email". Deleting a dictionary word forgets its anchors, and Reset App Data removes them.

## Numbers

All measured on this Mac through the full desktop Express path (Fluid Speech Mini ONNX, quiet-tail lift, Word Boost, the anchors, number formatting and the text pipeline), with "Eimailar" in the dictionary. Run the labs with the command at the top of `core/src/transcription/sound_lab.rs`.

### Your own voice (16 sentences)

`/Volumes/MyExtSSD/Developer/OwnVoice/orttakey-sound`: 10 sentences where you say Eimailar, 6 with plain "email". The recordings stay on the Mac, outside git.

| Stage | Eimailar shown | Plain "email" changed | iPhone |
|---|---|---|---|
| Before any fix (starting guess only) | **10/10** | **0/6** | 8/10, 0/6 |
| After one fix | 10/10 | 0/6 | 8/9, 0/6 |
| After one fix and 3 "email"s left as they were | 10/10 | 0/3 | 9/9, 0/3 |

- Nothing came out wrong before any fix, so there was no fix to make in the last two rows.
- 8 of the 10 were written by the spelling rule: the desktop model wrote "emailer" or "Emailer" (once Word Boost itself wrote Eimailar). The other 2 were written "email" and decided by the sound, at margins +3.9 and +4.4.
- Your 6 plain "email"s scored −13.1 to −10.1.

**Without the starting guess** (what a word gets when it doesn't look like an everyday word plus an ending):

| Stage | Eimailar shown | Plain "email" changed |
|---|---|---|
| Before any fix (Word Boost alone, as shipped in 0.7.1) | 1/10 | 0/6 |
| After one fix ("emailer" retyped as Eimailar) | 9/9 | 0/6 |
| After that fix and 3 "email"s left as they were | 9/9 | 0/3 |

### The 50 real-voice sentences (`gauntlet/asr_eval/corpus_real`)

With the starting guess and a learned fix on: **0 of 50 transcripts changed**. The two sentences with plain "email" were kept, at margins −9.3 and −5.7 (iPhone: −5.5 and −5.7).

### Synthetic voices (the iPhone lab's 720 clips)

10 macOS voices, 6 sentences saying "emailer", 12 saying plain "email" (6 of them hard ones such as "an email or a text"), at four speeds. Starting guess only, no fixes.

| Speed (words a minute) | Eimailar shown | by spelling | by sound | by slow listen | Plain "email" changed | iPhone shown |
|---|---|---|---|---|---|---|
| 170 | 38/60 | 33 | 1 | 4 | **0/120** | 39/60 |
| 200 | 35/60 | 28 | 4 | 3 | **0/120** | 41/60 |
| 260 | 31/60 | 20 | 6 | 5 | **0/120** | 31/60 |
| 320 | 28/60 | 15 | 2 | 11 | **0/120** | 32/60 |

This is also the check of the WSOLA stretch, by what the speech model hears in it: the slow listen wrote your word in 23 clips and changed none of the 480 plain ones.

### Speed

On this Mac (M5 Max, 4 threads), a 5-second dictation took about 350 ms without anchors. With them:

- **No "email" in it:** no measurable change.
- **"email" kept:** about 1 second, because the two slow listens each decode again. On a slower PC expect more.

The slow listen's catches had margins from −14.9 to −0.4, the same range as plain "email" (−13.6 to −2.7). So there is no margin below which it can be skipped.

## Different from the iPhone, and why

- **A guess stays unless a fix is about the same heard word.** On the iPhone a taught anchor replaces every guess for its word. Here a fix about "emails" would then take away what the guess knows about "email".
- **A sound spelling fixed to your word is learned as a spelling**, not as an anchor for "emailer" scored against "emailerer". In the own-voice lab the second way took the count from 1/10 to 0/9; the first takes it to 9/9.
- **A slow listen you change back is not tried for that pair again**, and the line learns from the sound check's own margin instead of the marker value 50.
- **No word-pair weighing.** The iPhone weighs "email at" with the keyboard's word-pair list at a bar where it changed nothing. The desktop has no such list, so an everyday ending word is never taken along.
- **Ordinary words only.** A fix is learned by sound when the heard words are English words. Otherwise the existing suggestion flow handles it.

## Not done, and why

- **Whisper.** There is no CTC model beside Whisper, so the sound can't be scored. A fix there still becomes a dictionary suggestion, as before.
- **Apps whose text can't be read.** Without the field's text Orttaai can't see your fix, so it learns nothing. The starting guess and the spelling rule still work.
- **Recordings over 2 minutes** aren't kept, so a fix in one isn't learned by sound. It becomes a dictionary suggestion as before; if you accept it, the pair is still decided by the sound when the word has an anchor.
- **Not run on a real Windows or Linux PC.** The Linux gate and the Windows build pass in Docker. The numbers above are from the Mac build of the same Rust code and the same ONNX models.

## Needs checking on a PC, with your voice

1. Add Eimailar to the dictionary with Express and Word Boost on. Say a few sentences with it, then a few with plain "email", including "email or a text".
2. With `ORTTAAI_LOG=info`, `orttaai.log` should show "sound anchors: N of M heard words written your way" for each.
3. Where one comes out "email", retype it as Eimailar and wait for the next dictation. The log should show "sound anchors: a fix taught how one of your words sounds".
4. Change one Eimailar back to "email". Memory › Suggestions must not offer a pair either way.
