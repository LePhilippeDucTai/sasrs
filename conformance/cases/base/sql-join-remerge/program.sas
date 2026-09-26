/* PROC SQL : jointure par WHERE + remerge GROUP BY — cf. PROC SQL documentation. */
libname ind 'data';

proc sql;
  create table joined as
    select c.name, o.amount
    from ind.customers as c, ind.orders as o
    where c.id = o.id;

  create table pct as
    select region, amount, amount / sum(amount) as share
    from ind.sales
    group by region;
quit;
