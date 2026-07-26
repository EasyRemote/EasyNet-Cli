Architecture
============

Domain
------

`ScheduleEntry` owns the prompt template as a required schedule fact. It is not
derived by the tick runner and not repaired by the store.

Ingress
-------

`schedule.add` validates prompt at the public ability boundary before building
`ScheduleCreateSpec`.

Persistence
-----------

The store validates schema facts before deserializing into domain objects. Old
records missing prompt, carrying null, or carrying blank prompt are obsolete
state and are skipped with a parse error.

Execution
---------

The tick runner renders `entry.prompt` directly. There is no heartbeat-style
placeholder branch.
