/* PROC MEANS CLASS + OUTPUT OUT= — cf. PROC MEANS documentation.
   Issue #14 : liste de stats stat=<var> (sum=/mean=) et OUT= sans mot-clé
   statistique (statistiques par défaut N MEAN STD MIN MAX). */
libname ind 'data';

proc means data=ind.sales noprint;
  class region;
  var amount;
  output out=summary sum=total_amount mean=avg_amount;
run;

proc means data=ind.sales noprint;
  class region;
  var amount;
  output out=default_stats;
run;
