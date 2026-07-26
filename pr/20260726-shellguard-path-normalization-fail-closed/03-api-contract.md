API contract
============

`PathVerdict`
-------------

- `Ok`: no write redirect violated policy.
- `Rejected`: write target normalized successfully but is outside allowed roots.
- `InvalidTarget`: write target could not be normalized into a concrete path.

Public behavior
---------------

`shell.run` continues to deny unsafe write redirects. The internal reason is
more precise for invalid target normalization.
