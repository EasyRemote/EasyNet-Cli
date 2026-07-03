# git-filter-repo --commit-callback body — round 2.
#
# Round-1 (tools/scripts/rewrite-rfc-messages.py) caught the
# "audit conversation / user-real audit / Per the user's instruction"
# class but missed second-person addressee ("You asked: ...",
# "your question", "the reviewer's framing"). This round cleans
# six commits. Replacement language uses fact-only subjects only —
# no You/your, no reviewer/audit/conversation as actor.
#
# Run:
#   git-filter-repo --commit-callback "$(cat tools/scripts/rewrite-rfc-messages-round2.py)" --force

REPLACEMENTS = {
    b"e5133f9": [
        (b"Audit follow-up to your question \\\"chat \xe6\x80\x8e\xe4\xb9\x88\xe4\xbc\xa0\xe5\x85\xa5\xe7\x89\xb9\xe5\xae\x9a ability/skill\ncontext?\\\". The answer wasn't documented anywhere; the smoke\nbinary now exercises every documented injection path so the\nreal wire-shape is empirically captured rather than asserted.",
         b"The answer to \"how does chat receive specific ability/skill\ncontext\" wasn't documented anywhere; the smoke binary now\nexercises every documented injection path so the real wire-\nshape is empirically captured rather than asserted."),
    ],

    b"207e88f": [
        (b"You asked: \\\"\xe9\x82\xa3 codex \xe8\x83\xbd\xe5\xa4\x9f\xe6\xb5\x81\xe5\xbc\x8f\xe8\xb0\x83\xe7\x94\xa8\xe5\x90\x97?\\\".\n\nPre-fix answer: NO. Slice 32 wired progress_tx through the",
         b"Question: does codex.chat actually stream?\n\nPre-fix answer: NO. Slice 32 wired progress_tx through the"),
    ],

    b"e163abf": [
        (b"You asked: \\\"\xe8\xbe\x93\xe5\x87\xba\xe7\x9a\x84\xe6\x98\xaf\xe6\xb5\x81\xe5\xbc\x8f\xe7\x9a\x84\xe5\x90\x97\xef\xbc\x9f\\\" The honest pre-fix answer was\n\\\"no\\\". stream_handler emitted exactly 3 frames per call:",
         b"Question: is the chat output actually streaming? The honest\npre-fix answer was \"no\". stream_handler emitted exactly 3\nframes per call:"),
    ],

    b"4937d90": [
        (b"You asked: \\\"\xe4\xbf\xae\xe5\xa4\x8d\xe4\xba\x86\xef\xbc\x9f\\\"\n\nThe honest answer was \\\"no\\\" for the deepest layer. Slice 28",
         b"Question: was this actually fixed? The honest answer was\n\"no\" for the deepest layer. Slice 28"),
    ],

    b"0c75ffd": [
        (b"You asked: \\\"besides reading, what about writing? executing\ncode? and the chat ability?\\\" The answer was \\\"yes\\\" for read\nin slice 22 only because it actually exercised that one path.\nThis slice does the same for the other four.",
         b"Question: besides reading, what about writing, executing code,\nand the chat ability? The answer was \"yes\" for read in slice 22\nonly because it actually exercised that one path. This slice\ndoes the same for the other four."),
    ],

    b"749e642": [
        (b"The reviewer's framing: \"you are defining EasyNet's process model,",
         b"The framing for v2: this defines EasyNet's process model,"),
    ],
}

short = commit.original_id[:7]
repls = REPLACEMENTS.get(short)
if repls:
    body = commit.message
    for needle, replacement in repls:
        if needle in body:
            body = body.replace(needle, replacement, 1)
        else:
            raise RuntimeError(
                "rewrite-rfc-messages-round2: needle not found in commit "
                + short.decode()
                + "; refusing partial rewrite"
            )
    commit.message = body
