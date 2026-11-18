module automatic m;
  function integer f(input integer v);
    integer acc = 1;
    return acc + v;
  endfunction
  function static integer g(input integer v);
    static integer acc = 1;
    acc = acc + v;
    return acc;
  endfunction
endmodule
