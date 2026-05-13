.. _safety:

Safety
======

ISO 26262 Safety Element out of Context (SEooC) safety concept for
sonic. Sketch-level coverage: assumed item, illustrative HARA, two
assumed safety goals, five assumed functional safety requirements
(AFSRs), ten technical safety requirements (TSRs) allocated to
sonic's existing crates, the Freedom From Interference argument
spanning spatial / temporal / information-exchange categories, and
the nine-item Assumption-of-Use (AoU) contract with the integrator.

**ASIL capability:** ASIL D, claimed via ISO 26262-9 §5 decomposition
``ASIL D = ASIL B(D) + ASIL B(D)``. Sonic is Element A at ASIL B(D);
the integrator supplies a diverse independent monitor as Element B
at ASIL B(D). The independence argument is claimed but not closed by
sonic — closure is an AoU on the integrator.

**How to read this section:**

1. Start with :doc:`item` — what sonic-hosted item we assume.
2. Read :doc:`hara` — assumed hazards and the safety goals they drive.
3. Read :doc:`decomposition` — how we get to ASIL D.
4. Read :doc:`fsc` for the assumed functional safety requirements,
   then :doc:`tsc` for the refinement onto sonic's crates.
5. Read :doc:`ffi` for the Freedom From Interference argument.
6. Read :doc:`aou` for what the integrator MUST validate.

Architecture decisions supporting this concept (ADR_0050, ADR_0051)
live in :doc:`../architecture/safety` under the architecture tree.

.. toctree::
   :maxdepth: 2
   :caption: Safety concept

   item
   hara
   decomposition
   fsc
   tsc
   ffi
   aou

Safety artefacts at a glance
----------------------------

.. needtable::
   :types: assumed-hazard
   :columns: id, title, status, asil
   :show_filters:

.. needtable::
   :types: assumed-safety-goal
   :columns: id, title, status, asil
   :show_filters:

.. needtable::
   :types: assumed-fsr
   :columns: id, title, status, asil, refines
   :show_filters:

.. needtable::
   :types: tsr
   :columns: id, title, status, asil, refines
   :show_filters:

.. needtable::
   :types: aou
   :columns: id, title, status
   :show_filters:
