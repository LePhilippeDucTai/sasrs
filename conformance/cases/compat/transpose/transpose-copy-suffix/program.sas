/* PROC TRANSPOSE — COPY et SUFFIX=.
   Cf. SAS 9.4 Procedures Guide, TRANSPOSE Procedure (COPY Statement) :
   « Because the COPY statement copies variables directly to the output data
   set, the number of observations in the output data set is equal to the
   number of observations in the input data set. The procedure pads the
   output data set with missing values if the number of observations in the
   input data set is not equal to the number of variables that it
   transposes. » SUFFIX= ajoute un suffixe au nom des variables transposees. */
libname ind 'data';

proc transpose data=ind.t out=out suffix=_s;
  var score;
  copy grader;
run;
