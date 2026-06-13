/* M9 : PROC RANK. Rangs de weight (ties=mean) puis quartiles d'age (groups=4). */
libname d 'data';

data class;
  set d.class;
run;

title 'Ranks of weight (ties=mean)';
proc rank data=class out=ranked ties=mean;
  var weight;
  ranks weight_rank;
run;

proc print data=ranked;
  var name weight weight_rank;
run;

title 'Age quartile groups (groups=4)';
proc rank data=class out=grouped groups=4;
  var age;
  ranks age_q;
run;

proc print data=grouped;
  var name age age_q;
run;
