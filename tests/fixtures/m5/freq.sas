/* M5 : PROC FREQ — tables 1 voie (sex, age) et 2 voies (sex*age). */
libname d 'data';

title 'One-way and two-way frequencies';
proc freq data=d.class;
  tables sex age;
  tables sex*age;
run;
