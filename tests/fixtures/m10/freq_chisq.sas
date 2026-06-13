/* M10 : FREQ chi-square sur une table 2x2 derivee + options NOROW/NOCOL.
   agegrp = Old si age>=14 sinon Yng. Table sex x agegrp :
   F: Old 4 / Yng 5 ; M: Old 5 / Yng 5 -> Pearson chi2 ~ 0.0587, ddl 1. */
libname d 'data';

data class2;
  set d.class;
  if age >= 14 then agegrp = 'Old';
  else agegrp = 'Yng';
run;

title 'Sex by age-group with chi-square (no row/col pct)';
proc freq data=class2;
  tables sex*agegrp / chisq norow nocol;
run;
