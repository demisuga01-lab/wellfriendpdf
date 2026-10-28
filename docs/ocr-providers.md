# OCR provider families

OCR provider selection is independent of the execution mode (Standard is the only mode). Hosted API and cloud document-intelligence providers are optional Standard capabilities: they require an explicitly configured and enabled provider, `resources.allow_external_network = true`, and a host policy that allows external network providers (the registry permissions are `providers.allow_hosted_api` and `providers.allow_cloud_document_intelligence`). The multi-provider fusion routing policy is `model_fusion` (`research_fusion` is a deprecated alias).

## Hosted API providers

Hosted API contracts support OpenAI-compatible vision endpoints and equivalent hosted OCR/VLM providers through configurable base URLs, model names, secret references, timeouts, retry policy, limits, cost hooks, and evidence metadata.

## Self-hosted providers

Self-hosted contracts cover Tesseract, PaddleOCR/PP-OCR-compatible providers, ONNX Runtime, OpenVINO, local OpenAI-compatible servers, optional TensorRT/CUDA, compact CPU OCR, and user plugins. The contract includes model identity, hash/version, session reuse, process isolation, memory budgets, batching, cancellation, language/script routing, and result caching.

## Cloud document intelligence

Cloud contracts cover Google, Azure, AWS Textract, and generic enterprise services through credential references, region endpoints, async jobs, polling, quotas, page limits, cancellation, timeout, cost metadata, and normalized text/layout/table/form geometry.

No external OCR or document-intelligence provider is invoked unless the operator explicitly configures and permits it.
