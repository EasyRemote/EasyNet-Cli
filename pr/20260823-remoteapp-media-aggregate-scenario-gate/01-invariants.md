# Invariants

1. Product completion requires baseline, degraded-network, and backpressure
   media scenarios.
2. Every media scenario summary must include negotiated video and audio codecs.
3. Every media scenario must include rendered video frames and rendered audio
   packets or samples.
4. Every media scenario must include a decoded render probe timestamp.
5. Degraded-network evidence must show lower target and observed bitrate than
   baseline, and either lower effective FPS or frame drops.
6. Backpressure evidence must include a backpressure event and more dropped
   frames than baseline.
7. Scenario summaries must bind the same selected Resource URA and media
   pipeline across the matrix.
8. Child media verifier reports still cannot emit `product_complete_claim=true`.
