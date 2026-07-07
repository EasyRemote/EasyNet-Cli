# Decisions

1. Treat `backend_ura`, `user_ura`, `session_id`, and `audiences` as retired
   session-authority DTO fields.
2. Use singular `audience` to match delegated authority and avoid list-specific
   semantics in the public session authority model.
3. Keep C ABI function names stable because they name the authority family, not
   the retired payload fields.
4. Remove Go session-authority Axon bridge helpers instead of translating the
   new DTO into the old Axon-generated payload.
5. Leave product session identifiers in Admin, Events, Bidi, and Wrappers
   untouched because those are different profile DTOs, not authority metadata.
