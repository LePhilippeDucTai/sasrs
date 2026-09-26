/* MERGE ... BY — appariement par clé, cf. MERGE Statement. */
libname ind 'data';

data merged;
  merge ind.customers ind.orders;
  by id;
run;
