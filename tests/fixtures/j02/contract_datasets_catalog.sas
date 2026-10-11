/* J02-P8 : contract of PROC DATASETS, CATALOG, FORMAT and GLM (no silent
   fallback). Each rejected step is followed by steps that still execute. */
data m;
  x = 1.23456; y = 2; z = 3;
  format z 5.1;
run;

title 'DATASETS: FORMAT inside MODIFY keeps the group';
proc datasets lib=work;
  modify m;
    format x 8.2 z;
    rename y=yy;
    label x='Ex';
quit;

proc contents data=m;
run;

proc print data=m;
run;

data a; v = 1; run;

title 'DATASETS: statements execute in source order';
proc datasets lib=work;
  change a=c;
  delete c;
quit;

proc datasets lib=work nolist;
  repair m;
quit;

proc format;
  value yn 1='Yes' 0='No';
run;

title 'CATALOG: the CAT= alias names the catalog';
proc catalog cat=work.formats;
  contents;
quit;

proc catalog catalog=work.formats et=format;
  contents;
quit;

proc format noreplace;
  value yn 1='Oui' 0='Non';
run;

data g;
  input grp $ blk $ y;
  datalines;
a x 1
a y 2
b x 4
b y 7
a x 2
b y 9
;
run;

proc glm data=g;
  class grp blk;
  model y = grp blk;
  estimate 'a-b' grp 1 -1;
run;

title 'GLM: one-way ESTIMATE, the E display option is ignored';
proc glm data=g;
  class grp;
  model y = grp;
  estimate 'a-b' grp 1 -1 / e;
run;
