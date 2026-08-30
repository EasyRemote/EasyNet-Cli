# RemoteApp bidi wire route contract intent

RemoteApp interactive desktop cannot be product-grade if the catalog only says
`call_mode = "bidi"` while omitting the concrete data-plane profile. Browser
and RemoteApp both use bidi, but browser attach uses JSON frames and RemoteApp
attach uses JSON metadata plus raw binary payload frames.

This slice makes the producer-owned descriptor publish the `bidi_wire_kind`
metadata so downstream product surfaces can select the correct stream adapter
without inferring from ability names.

