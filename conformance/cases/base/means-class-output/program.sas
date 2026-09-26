/* PROC MEANS CLASS + OUTPUT OUT= — cf. PROC MEANS documentation.
   Version sans liste de statistiques (voir divergence documentée). */
libname ind 'data';

proc means data=ind.sales noprint;
  class region;
  var amount;
  output out=summary;
run;
