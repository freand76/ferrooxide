// exit: 1
// An unpacked array result is no integral value.
module t;
  typedef int pair_t [2];
  function pair_t two(); two = '{1, 2}; endfunction
  int x;
  initial x = two() + 1;
endmodule
