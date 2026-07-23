# Architecture

`crate::core::ura` is the CLI boundary over Axon's canonical URA grammar. Realm
projection is a protocol fact, not a keyring or federation concern, so the
projection belongs in this facade.

Keyring remains responsible for signing and binding state. Federation remains
responsible for visibility policy. Neither owns a parallel URA grammar helper.
