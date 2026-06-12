/* M2 : etape DATA sans entree, ARRAY 1-D, DO iteratif, ** (puissance).
   s1..s5 = i**2 = 1,4,9,16,25 ; i vaut 6 a la sortie (regle SAS) ;
   total = 55. */
data work.squares;
  array a{5} s1-s5;
  do i = 1 to 5;
    a{i} = i ** 2;
  end;
  total = s1 + s2 + s3 + s4 + s5;
run;

proc print data=work.squares;
run;
