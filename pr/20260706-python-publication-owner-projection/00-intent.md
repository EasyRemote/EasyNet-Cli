# Intent

Remove owner-user inference from Python Publication catalogue URA string parsing.

`PublicationCatalogFacade.list_user()` may filter rows by catalog metadata or
Identity/Axon-projected owner components, but it must not understand the textual
encoding of user-owned Agent URAs. Publication owns catalogue projection; the
Directory + Identity profile owns URA projection.
