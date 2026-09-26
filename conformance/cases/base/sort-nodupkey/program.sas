/* PROC SORT NODUPKEY — cf. PROC SORT documentation. */
libname ind 'data';

proc sort data=ind.dups out=uniq nodupkey;
  by key;
run;
