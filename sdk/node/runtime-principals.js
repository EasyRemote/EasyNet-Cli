const ALL_ZERO_PRINCIPAL_ID = "00000000-0000-0000-0000-000000000000";

export function containsAllZeroPrincipal(value) {
  return String(value ?? "").trim().toLowerCase().includes(ALL_ZERO_PRINCIPAL_ID);
}
