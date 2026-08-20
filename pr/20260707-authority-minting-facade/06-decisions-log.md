# Decisions Log

## 2026-07-07

Decision: add an SDK authority minting facade over a transport/provider
boundary, instead of copying Axon canonical authority algorithms into Go or
Python.

Reason: backend import-ban must be solved by moving product callers onto the
CLI SDK boundary while Axon remains the protocol semantic owner.

Follow-up: concrete daemon/C ABI authority minting transport and EasyNet
backend source cutover remain separate capability slices. This commit adds the
public SDK boundary they will consume.
