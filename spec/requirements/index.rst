Requirements
============

System-level requirements for `sonic-executor` framed as the runtime heart
of a soft-real-time PLC. Each ``req`` directive ``:satisfies:`` one ``feat``
parent; each ``feat`` ``:satisfies:`` the top-level :need:`FEAT_0010`.

.. toctree::
   :maxdepth: 2

   plc-runtime

Requirements at a glance
------------------------

.. needtable::
   :types: req
   :columns: id, title, status, satisfies
   :show_filters:
