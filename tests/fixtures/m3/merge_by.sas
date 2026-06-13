/* M3 : MERGE par BY avec IN= — match-merge SAS.
   Deux sous-tables par name (eleves ages >= 14 ; eleves lourds >= 100) sont
   interclassees ; in_ages/in_weights tracent la provenance de chaque obs. */
libname d 'data';

data work.ages;
  set d.class(keep=name age);
  if age >= 14;
run;

data work.weights;
  set d.class(keep=name weight);
  if weight >= 100;
run;

proc sort data=work.ages;   by name; run;
proc sort data=work.weights; by name; run;

data work.both;
  merge work.ages(in=ina) work.weights(in=inb);
  by name;
  in_ages = ina;
  in_weights = inb;
run;

title 'Match-merge of older (14+) and heavier (100+) pupils';
proc print data=work.both;
run;
