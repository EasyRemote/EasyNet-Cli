# Intent

Hosted Agent purge and revoke recovery need one durable identity fence on the
Hub. The existing in-flight implementation already models generation-bound
revokes, but the Hub inventory lookup still treated `agent_ura` as the sole
slot identity in several paths.

This slice tightens the durable slot to `(agent_ura, authority_ura)`, carries
owner-projection generation through advertise payloads, and keeps local
`federation.revoke` targeted at the agent identity being retired.
