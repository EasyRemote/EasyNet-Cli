# Capability-native network README positioning

## Intent

Make the repository's category thesis explicit in the root README: EasyNet is
building a capability-native network whose basic unit is an authorized,
invocable, and verifiable Ability.

## Invariants

- Keep the claim grounded in the implemented Ability, Invocation, Admission,
  and Receipt model.
- Do not imply that EasyNet is a generic VPN, transport network, or file
  network.
- Preserve the existing Runtime positioning and public behavior.
- Do not expand the change beyond documentation.

## Architecture

The category thesis belongs above the Runtime description. The Runtime remains
the concrete implementation layer that gives an Ability identity, policy,
routing, execution, and terminal evidence.

## Checklist

- [x] Read the current README positioning and repository ownership guide.
- [x] Add the category thesis to the README hero.
- [x] Connect the thesis to the existing Runtime explanation.
- [x] Verify the rendered wording and diff.

## Decisions

- Use `Ability` rather than generic `capability` for the executable network
  object because it is the repository's public domain term.
- Keep the thesis to one sentence in the hero and one grounding sentence in
  the architecture narrative to avoid duplicating the existing explanation.

## Verification

- `git diff --check -- README.md` passed.
- The README hero defines the category and the architecture narrative grounds
  it in Ability, Invocation, and Receipt ownership.
