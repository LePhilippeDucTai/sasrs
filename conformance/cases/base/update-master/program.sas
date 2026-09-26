/* UPDATE maître/transactions — cf. UPDATE Statement. */
libname ind 'data';

data master;
  update ind.master ind.trans;
  by id;
run;
