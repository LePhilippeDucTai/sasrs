/* M7 : OPTIONS FIRSTOBS=/OBS= (fenetre d'entree de l'etape DATA) + LAG/DIF,
   et les fonctions de dates INTNX/INTCK. */
libname d 'data';

options firstobs=2 obs=5;
data work.win;
  set d.class(keep=name age);
  prev_age = lag(age);
  age_diff = dif(age);
run;
options firstobs=1 obs=max;

title 'FIRSTOBS=2 OBS=5 window with LAG/DIF';
proc print data=work.win;
run;

data work.dates;
  start = '15jan2020'd;
  next_month = intnx('month', start, 1);
  months_between = intck('month', '15jan2020'd, '10mar2020'd);
run;

title 'INTNX / INTCK';
proc print data=work.dates;
run;
