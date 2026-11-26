// exit: 1
// A function result of several unpacked dimensions is not supported (yet).
module t;
  typedef byte grid_t [2][3];
  function automatic grid_t grid();
    foreach (grid[i, j]) grid[i][j] = 8'(10 * i + j);
  endfunction
  grid_t g;
  initial g = grid();
endmodule
