Architecture
============

Detailed-design specifications. Pages under this section follow the
arc42 template (12 sections) encoded with sphinx-needs using the
useblocks "x-as-code" arc42 directive types — ``arch-decision``,
``building-block``, ``architecture``, ``constraint``, ``quality-goal``,
``risk``, ``glossary``. Legacy ``spec`` directives may also appear for
detailed-design notes that predate the arc42 adoption.

.. toctree::
   :maxdepth: 2

   connector

Building blocks
---------------

.. needtable::
   :types: building-block
   :columns: id, title, status, implements
   :show_filters:

Architecture views (context, runtime, deployment, crosscutting)
---------------------------------------------------------------

.. needtable::
   :types: architecture
   :columns: id, title, status, refines
   :show_filters:

Architecture decisions
----------------------

.. needtable::
   :types: arch-decision
   :columns: id, title, status, refines
   :show_filters:

Quality goals and constraints
-----------------------------

.. needtable::
   :types: quality-goal
   :columns: id, title, status, refines
   :show_filters:

.. needtable::
   :types: constraint
   :columns: id, title, status, refines
   :show_filters:

Risks
-----

.. needtable::
   :types: risk
   :columns: id, title, status, links
   :show_filters:

Glossary
--------

.. needtable::
   :types: term
   :columns: id, title, status
   :show_filters:

Legacy detailed-design specifications
-------------------------------------

.. needtable::
   :types: spec
   :columns: id, title, status, refines
   :show_filters:
