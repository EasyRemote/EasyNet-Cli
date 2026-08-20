# Boundary Proof

Runtime trust owns principal-owner facts for devices admitted by
`federation.join`. Hosted Agent publication reads those facts; it must not infer
or repair missing owner aliases at publication time.

The state boundary is:

1. parse and verify `principal_enrollment.principal_ura` as a User URA in the
   join realm;
2. verify the enrollment proof through `PrincipalLifecycle`;
3. persist one `TrustedPrincipalOwner` for the joining membership URA;
4. carry the authenticated user id into both `owner_user_id` and
   `owner_username` for this proof shape;
5. let downstream publication admission consume the stored owner binding
   without a fallback path.

This preserves the public `federation.join` input shape while making the trust
projection internally complete.
