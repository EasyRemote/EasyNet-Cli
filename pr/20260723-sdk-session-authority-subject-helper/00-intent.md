# Intent

Remove the Python SDK's local session-authority subject admission wrapper from
`authorized_runtime_session.py`.

The canonical SDK subject-admission rule is owned by
`easynet_sdk._session_authority_subjects.session_authority_admits_subject`.
Authorized runtime sessions and runtime ability validation should call that
same helper directly so history authorization, prepare/resolve authorization,
and runtime ability validation cannot drift.
