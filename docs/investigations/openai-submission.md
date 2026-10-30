# OpenAI submission preparation

Working notes for the OpenAI item in `TODO.md`; remove this file and its inbound
links when that item closes. The package contract and verified client scope live in
[`docs/oauth.md`](../oauth.md).

## Last checked: 2026-10-30

- The identity check was submitted; the dashboard showed **Identity in review**
  and blocked ZIP uploads. The package has not been uploaded or portal-validated.
- The portable ZIP includes the required developer name and composer icon,
  plus the [five positive and three negative cases](../../contrib/openai/review-cases.json).
  Local package, transport and OAuth tests passed; these eight model prompts
  have not been run with a reviewer account in ChatGPT or Codex.
- No review account, reviewer OAuth binding or demo deployment was created.

## Still needed before public submission

1. Choose reviewer access that exposes only synthetic data. A new seat on the
   real bus is not isolation: `exclude_client_data` still exposes global
   knowledge. Keep credentials and sign-in instructions in the portal's secure
   review details, outside the package and repository.
2. Once identity approval clears, build the `openai` ZIP for the chosen review
   endpoint using `--url`, upload it and resolve the actual portal findings.
   Rebuild if the endpoint changes; a successful local build is not portal
   acceptance.
3. Finish the website, support and terms URLs, demo recording and release notes.
   The package already declares a privacy-policy URL. Do not invent service terms
   or assume the software license covers the hosted service.
4. Run all eight prompts in the target hosts and record actual tool calls and
   outcomes with the reviewer account. Complete domain verification and the
   authenticated MCP scan against the chosen review endpoint.

The [OpenAI submission guide](https://developers.openai.com/plugins/deploy/submission)
defines the required fields and review materials.

## Synthetic case setup

Use seats `Review-Worker` and `Review-Peer` in the chosen synthetic-only review
environment. Bind the reviewer's OAuth identity to `Review-Worker`; allow it to
read its own handoffs and write global knowledge.

Seed a pending action from `Review-Peer` to `Review-Worker` titled
`[Demo] Acknowledge receipt`, whose entire task is to acknowledge receipt.
Also seed a completed action from `Review-Worker` to `Review-Peer` titled
`[Demo] Completed lookup`, with a notify reply titled
`[Demo] Lookup acknowledgement` containing `Sample status confirmed.` and
linked by `in_reply_to`. Reset only this synthetic data before a fresh run.
The peer need not be online: creating a handoff queues work.
