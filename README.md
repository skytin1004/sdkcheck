# sdkcheck

sdkcheck is a Rust CLI that audits whether an agent can follow your product docs, install your product, and complete the intended flow.

It runs the documented setup in an isolated environment and writes an evidence report when the flow breaks.

## Quick Start

```bash
sdkcheck run \
  --repo https://github.com/acme/product.git \
  --goal "Install the product and complete the quickstart." \
  --agent-base-url http://localhost:4000/v1 \
  --agent-model gpt-4.1-mini \
  --agent-api-key-env ANY_LLM_API_KEY
```
