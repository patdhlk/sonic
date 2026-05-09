Overview
========

`sonic-executor` is a Rust execution framework that turns iceoryx2 IPC events,
intervals, and request/response activity into deterministic, observable
schedules of executable items. It supports sequential chains, parallel DAGs,
signal/slot composition, and lifecycle observability.

This specification documents the framework as engineering artefacts —
features, requirements, architecture elements, and verification — using
`sphinx-needs`_ directives. Each artefact has a stable ID and traceable links
to the artefacts it satisfies, refines, implements, or verifies.

.. _sphinx-needs: https://sphinx-needs.com

Stub features
-------------

The features below are placeholders to confirm the build pipeline works.
Real feature derivation lands in a follow-up commit; the eventual flow is
``pharaoh:write-plan`` (template ``reverse-engineer-project``) →
``pharaoh:execute-plan``, which mines the Rust source under ``../crates/``
and emits ``feat`` + ``req`` directives populated from the implementation.

.. feat:: Triggered execution
   :id: FEAT_0001
   :status: open

   Items execute when one or more of their declared triggers fire — pub/sub
   subscriptions, intervals, deadlines, request/response endpoints, or raw
   iceoryx2 listeners.

.. feat:: Composition
   :id: FEAT_0002
   :status: open

   Items compose into sequential chains and parallel DAGs with explicit
   abort semantics on ``StopChain`` or ``Err`` returns.

.. feat:: Observability
   :id: FEAT_0003
   :status: open

   Lifecycle and per-execution timing visibility via the ``Observer`` and
   ``ExecutionMonitor`` traits, with a ready-made tracing-crate adapter.
