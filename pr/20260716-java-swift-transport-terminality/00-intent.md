# Intent

Align Java and Swift stream/bidi facades with receipt-backed terminality.

Transport events such as cancel, close-send and backpressure stop local SDK
delivery, but they are not runtime terminal receipts. This slice separates
transport-terminal state from receipt-backed terminal state in the Java and
Swift SDK seams.
