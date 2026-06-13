/* M3 : PROC SORT — OUT=, BY avec DESCENDING, NODUPKEY.
   Tri par sex (asc) puis age (desc) ; NODUPKEY garde une obs par couple
   (sex, age) et NOTE le nombre de doublons supprimes. */
libname d 'data';

proc sort data=d.class out=work.bysex nodupkey;
  by sex descending age;
run;

title 'First pupil per (sex, age), oldest first within sex';
proc print data=work.bysex;
run;
