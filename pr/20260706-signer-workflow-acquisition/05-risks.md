Risks:
- Import cycles in Python if IdentityClient imports signing at module load.
- Mitigation: import Signer inside acquire_signer.
- The helper must not imply daemon keyring policy is complete.
- Mitigation: parity docs keep daemon keyring policy as remaining work.
