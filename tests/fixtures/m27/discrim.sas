title 'PROC DISCRIM: linear discriminant analysis';
data work.lda;
  input class $ x;
datalines;
A 1
A 2
A 3
B 5
B 6
B 7
;
proc discrim data=work.lda;
  class class;
  var x;
run;
title;
