/* PROC FORMAT VALUE + PUT — cf. PROC FORMAT documentation. */
libname ind 'data';

proc format;
  value grade 0-59 = 'F'
              60-79 = 'C'
              80-100 = 'A';
run;

data report;
  set ind.scores;
  length grade $ 1;
  grade = put(score, grade.);
run;
