# OCR provider families

OCR provider selection is orthogonal to execution mode.

## Hosted API providers

Hosted API contracts support OpenAI-compatible vision endpoints and equivalent hosted OCR/VLM providers through configurable base URLs, model names, secret references, timeouts, retry policy, limits, cost hooks, and evidence metadata.

## Self-hosted providers

Self-hosted contracts cover Tesseract, PaddleOCR/PP-OCR-compatible providers, ONNX Runtime, OpenVINO, local OpenAI-compatible servers, optional TensorRT/CUDA, compact CPU OCR, and user plugins. The contract includes model identity, hash/version, session reuse, process isolation, memory budgets, batching, cancellation, language/script routing, and result caching.

## Cloud document intelligence

Cloud contracts cover Google, Azure, AWS Textract, and generic enterprise services through credential references, region endpoints, async jobs, polling, quotas, page limits, cancellation, timeout, cost metadata, and normalized text/layout/table/form geometry.

No external OCR or document-intelligence provider is invoked unless the operator explicitly configures and permits it.
