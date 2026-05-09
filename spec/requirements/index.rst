Requirements
============

System-level requirements. The spec is organised under two peer top-level
features:

* :need:`FEAT_0010` "PLC runtime heart on iceoryx2" — sonic-executor
  framed as the runtime heart of a soft-real-time PLC. See
  :doc:`plc-runtime`.
* :need:`FEAT_0030` "Connector framework" — the general-purpose framework
  for bridging sonic-executor applications to external protocols. See
  :doc:`connector`.

Each ``req`` directive ``:satisfies:`` one ``feat`` parent; each
capability-cluster ``feat`` ``:satisfies:`` its top-level umbrella feature.

.. toctree::
   :maxdepth: 2

   plc-runtime
   connector

Requirements at a glance
------------------------

.. needtable::
   :types: req
   :columns: id, title, status, satisfies
   :show_filters:
