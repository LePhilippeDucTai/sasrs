/* UPCASE, SUBSTR, TRIM/CATS, MDY, YEAR — cf. Functions and CALL Routines. */
libname ind 'data';

data words;
  set ind.names;
  length upper $ 5 short $ 3 both $ 6;
  upper = upcase(name);
  short = substr(name, 1, 3);
  both = cats(trim(name), '!');
  yr = year(mdy(month, 15, 2020));
run;
